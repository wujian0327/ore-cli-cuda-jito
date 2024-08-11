use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use drillx::{equix, Hash, Solution};
use ore_api::{
    consts::{BUS_ADDRESSES, BUS_COUNT},
    state::Proof,
};
use rand::Rng;
use solana_program::pubkey::Pubkey;
use solana_rpc_client::spinner;
use solana_sdk::signer::Signer;
use tokio::task;

use crate::{
    args::MineArgs,
    utils::{amount_u64_to_string, get_config, get_proof_with_authority, proof_pubkey},
    Miner,
};

impl Miner {
    pub async fn mine_cuda(&self, args: MineArgs) {
        // Register, if needed.
        let signer = self.signer();
        self.open().await;

        // Start mining loop
        loop {
            // Fetch proof
            let proof = get_proof_with_authority(&self.rpc_client, signer.pubkey()).await;
            println!(
                "\nStake balance: {} ORE",
                amount_u64_to_string(proof.balance)
            );

            // Calc cutoff time
            let cutoff_time = self.get_cutoff(proof, args.buffer_time).await;

            // Run drillx
            let config = get_config(&self.rpc_client).await;

            //  结束标志flag
            let finish_flag = Arc::new(Mutex::new(false));
            let gpu_nonce = Arc::new(Mutex::new(0 as u64));
            let cpu_nonce = Arc::new(Mutex::new(0 as u64));
            let gpu_nonce_clone = gpu_nonce.clone();
            let cpu_nonce_clone = cpu_nonce.clone();
            let finish_flag_clone = finish_flag.clone();

            //cuda task
            let task1 = task::spawn(async move {
                let proof_clone = proof.clone();
                Self::find_hash_par_cuda(
                    proof_clone,
                    cutoff_time,
                    args.min as u32,
                    args.size as i32,
                    gpu_nonce_clone,
                    finish_flag_clone,
                );
            });

            //cpu task
            let task2 = task::spawn(async move {
                Self::find_hash_par_cpu(
                    proof,
                    cutoff_time,
                    args.threads,
                    args.min as u32,
                    cpu_nonce_clone,
                    finish_flag,
                );
            });

            let _ = tokio::join!(task1, task2);

            let gpu_nonce_clone = gpu_nonce.clone();
            let cpu_nonce_clone = cpu_nonce.clone();
            let mut memory = equix::SolverMemory::new();
            let hx1 = drillx::hash_with_memory(
                &mut memory,
                &proof.challenge,
                &gpu_nonce_clone.lock().unwrap().to_le_bytes(),
            )
            .unwrap();
            let mut memory = equix::SolverMemory::new();
            let hx2 = drillx::hash_with_memory(
                &mut memory,
                &proof.challenge,
                &cpu_nonce_clone.lock().unwrap().to_le_bytes(),
            )
            .unwrap();

            let nonce1 = gpu_nonce.lock().unwrap();
            let nonce2 = cpu_nonce.lock().unwrap();
            let nonce = if hx1.difficulty() > hx2.difficulty() {
                nonce1
            } else {
                nonce2
            };

            let mut memory = equix::SolverMemory::new();
            let hx = drillx::hash_with_memory(&mut memory, &proof.challenge, &nonce.to_le_bytes())
                .unwrap();
            let sol = Solution::new(hx.d, nonce.to_le_bytes());

            println!(
                "Best hash: {} (difficulty: {})",
                bs58::encode(hx.h).into_string(),
                hx.difficulty()
            );

            // Submit most difficult hash
            // let mut compute_budget = 500_000;
            let mut ixs = vec![ore_api::instruction::auth(proof_pubkey(signer.pubkey()))];
            if self.should_reset(config).await {
                // compute_budget += 100_000;
                ixs.push(ore_api::instruction::reset(signer.pubkey()));
            }
            ixs.push(ore_api::instruction::mine(
                signer.pubkey(),
                signer.pubkey(),
                find_bus(),
                sol,
            ));
            self.send_and_confirm_d_jito(&ixs).await;
        }
    }

    fn find_hash_par_cuda(
        proof: Proof,
        cutoff_time: u64,
        min_difficulty: u32,
        size: i32,
        gpu_nonce: Arc<Mutex<u64>>,
        finish_flag: Arc<Mutex<bool>>,
    ) {
        println!("GPU Mining...");
        let mut nonce = u64::MAX / 2;
        let mut best_nonce = nonce;
        let timer = Instant::now();
        let batch_size = 512 * size;
        let mut best_difficulty = 0;
        loop {
            if *finish_flag.lock().unwrap() {
                break;
            }
            let (best_n, best_diff) =
                drillx_cuda::cuda::hash_with_cuda(proof.challenge, nonce, batch_size);
            if best_diff > best_difficulty {
                best_difficulty = best_diff;
                best_nonce = best_n;
                println!("GPU Best difficulty: {}, nonce:{})", best_diff, best_n);
            }
            nonce = nonce + batch_size as u64;
            println!(
                "GPU Mining... ({} sec remaining)",
                cutoff_time.saturating_sub(timer.elapsed().as_secs())
            );
            if timer.elapsed().as_secs().ge(&cutoff_time) {
                if best_difficulty.gt(&min_difficulty) {
                    break;
                }
            }
        }
        *gpu_nonce.lock().unwrap() = best_nonce;
        println!(
            "GPU Best difficulty: {}, nonce:{})",
            best_difficulty, best_nonce
        );
        *finish_flag.lock().unwrap() = true;
    }

    fn find_hash_par_cpu(
        proof: Proof,
        cutoff_time: u64,
        threads: u64,
        min_difficulty: u32,
        cpu_nonce: Arc<Mutex<u64>>,
        finish_flag: Arc<Mutex<bool>>,
    ) {
        // Dispatch job to each thread
        let bool_value = Arc::new(Mutex::new(false));
        let progress_bar = Arc::new(spinner::new_progress_bar());
        println!("CPU Mining...");
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                std::thread::spawn({
                    let proof = proof.clone();
                    let progress_bar = progress_bar.clone();
                    let mut memory = equix::SolverMemory::new();
                    let bool_value_clone = bool_value.clone();
                    let finish_flag_clone = finish_flag.clone();
                    move || {
                        let timer = Instant::now();
                        let mut nonce = ((u64::MAX) / 2).saturating_div(threads).saturating_mul(i);
                        let mut best_nonce = nonce;
                        let mut best_difficulty = 0;
                        let mut best_hash = Hash::default();
                        loop {
                            // Create hash
                            if let Ok(hx) = drillx::hash_with_memory(
                                &mut memory,
                                &proof.challenge,
                                &nonce.to_le_bytes(),
                            ) {
                                let difficulty = hx.difficulty();
                                if difficulty.gt(&best_difficulty) {
                                    best_nonce = nonce;
                                    best_difficulty = difficulty;
                                    best_hash = hx;
                                }
                            }

                            // Exit if time has elapsed
                            if nonce % 100 == 0 {
                                if timer.elapsed().as_secs().ge(&cutoff_time) {
                                    if *finish_flag_clone.lock().unwrap() {
                                        break;
                                    }
                                    if *bool_value_clone.lock().unwrap() {
                                        break;
                                    }
                                    if best_difficulty.gt(&min_difficulty) {
                                        *bool_value_clone.lock().unwrap() = true;
                                        // Mine until min difficulty has been met
                                        break;
                                    }
                                } else if i == 0 {
                                    progress_bar.set_message(format!(
                                        "Mining... ({} sec remaining)",
                                        cutoff_time.saturating_sub(timer.elapsed().as_secs()),
                                    ));
                                }
                            }

                            // Increment nonce
                            nonce += 1;
                        }

                        // Return the best nonce
                        (best_nonce, best_difficulty, best_hash)
                    }
                })
            })
            .collect();

        // Join handles and return best nonce
        let mut best_nonce = 0;
        let mut best_difficulty = 0;
        for h in handles {
            if let Ok((nonce, difficulty, _hash)) = h.join() {
                if difficulty > best_difficulty {
                    best_difficulty = difficulty;
                    best_nonce = nonce;
                }
            }
        }
        *cpu_nonce.lock().unwrap() = best_nonce;
        println!(
            "CPU Best difficulty: {}, nonce:{})",
            best_difficulty, best_nonce
        );
        *finish_flag.lock().unwrap() = true;
    }
}

// TODO Pick a better strategy (avoid draining bus)
fn find_bus() -> Pubkey {
    let i = rand::thread_rng().gen_range(0..BUS_COUNT);
    BUS_ADDRESSES[i]
}
