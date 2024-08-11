#include <stdint.h>
#include <stdio.h>
#include "drillx.h"
#include "equix.h"
#include "hashx.h"
#include "equix/src/context.h"
#include "equix/src/solver.h"
#include "equix/src/solver_heap.h"
#include "hashx/src/context.h"

// const int BATCH_SIZE = 512 * 1;

extern "C" void hash(uint8_t *challenge, uint8_t *nonce, uint8_t *digest, int batch_size)
{
    // Generate a hash function for each (challenge, nonce)
    hashx_ctx **ctxs;
    if (cudaMallocManaged(&ctxs, batch_size * sizeof(hashx_ctx *)) != cudaSuccess)
    {
        printf("Failed to allocate managed memory for ctxs\n");
        return;
    }
    uint8_t seed[40];
    memcpy(seed, challenge, 32);
    for (int i = 0; i < batch_size; i++)
    {
        uint64_t nonce_offset = *((uint64_t *)nonce) + i;
        memcpy(seed + 32, &nonce_offset, 8);
        ctxs[i] = hashx_alloc(HASHX_INTERPRETED);
        if (!ctxs[i] || !hashx_make(ctxs[i], seed, 40))
        {
            // printf("Failed to make hash\n");
        }
    }

    // Allocate space to hold on to hash values (~500KB per seed)
    uint64_t **hash_space;
    if (cudaMallocManaged(&hash_space, batch_size * sizeof(uint64_t *)) != cudaSuccess)
    {
        printf("Failed to allocate managed memory for hash_space\n");
        return;
    }
    for (int i = 0; i < batch_size; i++)
    {
        if (cudaMallocManaged(&hash_space[i], INDEX_SPACE * sizeof(uint64_t)) != cudaSuccess)
        {
            printf("Failed to allocate managed memory for hash_space[%d]\n", i);
            return;
        }
    }

    // Launch kernel to parallelize hashx operations
    dim3 threadsPerBlock(256);                                                            // 256 threads per block
    dim3 blocksPerGrid((65536 * batch_size + threadsPerBlock.x - 1) / threadsPerBlock.x); // enough blocks to cover batch
    do_hash_stage0i<<<blocksPerGrid, threadsPerBlock>>>(ctxs, hash_space, batch_size);
    cudaDeviceSynchronize();

    // equix_ctx
    equix_ctx **eq_ctxs;
    if (cudaMallocManaged(&eq_ctxs, batch_size * sizeof(equix_ctx *)) != cudaSuccess)
    {
        printf("Failed to allocate managed memory for equix_ctx\n");
    }
    for (int i = 0; i < batch_size; i++)
    {
        eq_ctxs[i] = equix_alloc(EQUIX_CTX_SOLVE);

        if (eq_ctxs[i] == nullptr)
        {
            printf("Failed to allocate equix context\n");
            return;
        }
        else
        {
            eq_ctxs[i]->hash = hash_space[i];
        }
    }
    // digest
    uint8_t *fp_device_digest;
    cudaMalloc((float **)&fp_device_digest, batch_size * sizeof(uint8_t) * 16);
    if (fp_device_digest != NULL)
    {
        cudaMemset(fp_device_digest, 0, batch_size * sizeof(uint8_t) * 16);
    }

    do_remain_stage<<<batch_size / 128, 128>>>(eq_ctxs, fp_device_digest, batch_size);
    cudaDeviceSynchronize();

    // copy to host
    cudaMemcpy(digest, fp_device_digest, batch_size * sizeof(uint8_t) * 16, cudaMemcpyDeviceToHost);

    // Free memory
    for (int i = 0; i < batch_size; i++)
    {
        hashx_free(ctxs[i]);
        equix_free(eq_ctxs[i]);
        cudaFree(hash_space[i]);
    }
    cudaFree(hash_space);
    cudaFree(fp_device_digest);
    cudaFree(ctxs);
    cudaFree(eq_ctxs);

    // Generate a hash function for each (challenge, nonce)

    // Print errors
    cudaError_t err = cudaGetLastError();
    if (err != cudaSuccess)
    {
        printf("CUDA error: %s\n", cudaGetErrorString(err));
        printf("Error at file:%s, line:%d\n", __FILE__, __LINE__);
    }
}

__global__ void do_hash_stage0i(hashx_ctx **ctxs, uint64_t **hash_space, int batch_size)
{
    uint32_t item = blockIdx.x * blockDim.x + threadIdx.x;
    uint32_t batch_idx = item / INDEX_SPACE;
    uint32_t i = item % INDEX_SPACE;
    if (batch_idx < batch_size)
    {
        hash_stage0i(ctxs[batch_idx], hash_space[batch_idx], i);
    }
}

__global__ void do_remain_stage(equix_ctx **ctxs, uint8_t *digest, int batch_size)
{
    uint32_t i = blockIdx.x * blockDim.x + threadIdx.x;

    if (i < batch_size)
    {
        equix_ctx *ctx = ctxs[i];
        equix_solution solutions[EQUIX_MAX_SOLS];
        uint32_t num_sols = equix_solver_solve(ctx->hash, ctx->heap, solutions);

        if (num_sols > 0)
        {
            memcpy(digest + (i * 16), solutions[0].idx, sizeof(solutions[0].idx));
        }
        else
        {
            memset(digest + (i * 16), 0, sizeof(solutions[0].idx));
        }
    }
}
