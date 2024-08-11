extern "C" {
    pub fn hash(challenge: *const u8, nonce: *const u8, digest: *mut u8, batch_size: i32);
}

pub fn hash_with_cuda(challenge: [u8; 32], nonce: u64, batch_size: i32) -> (u64, u32) {
    let digestspace = 16 * batch_size as usize;
    let challenge = challenge.clone();
    let nonce = nonce.to_le_bytes();
    let mut digest: Vec<u8> = vec![0u8; digestspace];
    let mut best_nonce = u64::from_le_bytes(nonce);
    let mut best_difficulty = 0;
    unsafe {
        // Do compute heavy hashing on gpu
        // let timer = Instant::now();
        hash(
            challenge.as_ptr(),
            nonce.as_ptr(),
            digest.as_mut_ptr() as *mut u8,
            batch_size,
        );
        // println!(
        //     "Gpu returned {} hashes in {} ms",
        //     batch_size,
        //     timer.elapsed().as_millis()
        // );
        // let timer = Instant::now();
        for i in 0..batch_size as usize {
            let nonce = u64::from_le_bytes(nonce);
            let mut merged_vec = [0u8; 16];
            for j in 0..16 {
                let a = digest.get(i * 16 + j).unwrap().clone();
                merged_vec[j] = a;
            }
            if merged_vec != [0 as u8; 16] {
                let solution = drillx::Solution::new(merged_vec, (nonce + i as u64).to_le_bytes());
                let hx = solution.to_hash();
                let difficulty = hx.difficulty();
                if difficulty.gt(&best_difficulty) {
                    best_nonce = nonce + i as u64;
                    best_difficulty = difficulty;
                }
            }
        }
        // println!(
        //     "CPU returned {} hashes in {} ms",
        //     batch_size,
        //     timer.elapsed().as_millis()
        // );
    }
    (best_nonce, best_difficulty)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_gpu() {
        let challenge = [155; 32];
        let nonce = 1;
        let (best_nonce, best_difficult) = hash_with_cuda(challenge, nonce, 512 * 2);
        let l = format!("Best difficulty: {}, nonce:{})", best_difficult, best_nonce);
        println!("{}", l);
        //check
        let hx = drillx::hash(&challenge, &best_nonce.to_le_bytes()).unwrap();
        // let hx = crate::hash(&challenge, &best_nonce.to_le_bytes()).unwrap();
        println!("drillx difficulty:{}", hx.difficulty());
    }
}
