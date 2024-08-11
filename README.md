# ore-cli-cuda-jito

What you need to know is that

**the current GPU performance is much lower than the CPU.**

Copy from https://github.com/regolith-labs/drillx.git

I tested the 4090 and A100, and the performance is not as good as the average desktop.

**No optimization.** Just for fun.

You can use gpu and cpu to calculate hashes at the same time, and then use jito to submit your transaction.

## System requirements

Ubuntu22

You need to install:

NVCC，Protoc，Rust

## Jito requirements

you should get a Solana public key [approved](https://jito-labs.gitbook.io/mev/searcher-services/shredstream#how-do-i-sign-up) in jito, and this public key is not the same as your ore submit public key.

[Getting Started | Jito (gitbook.io)](https://jito-labs.gitbook.io/mev/searcher-resources/getting-started)

This key is used for jito_auth.json.

And jito_auth.json will pay gas fees.

## Get start

```shell
cargo build --release
```

```shell
cargo run --release   -- mine-cuda --keypair ./id.json --rpc RPC_URL  --priority-fee 0 --min 10 --threads 10  --jito-fee 5000 --jito-auth jito_auth.json --size 1
```

The size parameter is the number of hashes that the GPU processes at one time. If it is too large, the gpu memory will not be enough.

or

change the start.sh and run `./start.sh`



my laptop 3060，12700h

![image-20240811143156359](https://gitee.com/wujian2023/typora_images/raw/master/auto_upload/image-20240811143156359.png)



