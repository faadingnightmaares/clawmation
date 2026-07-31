# Hybrid GPU Vision Design

## Goal

Accelerate expensive template-correlation searches without changing Clawmation's
thresholds, scale ladder, coordinates, detection ordering, or user-facing
behavior.

## Architecture

- Keep the existing Rust CPU matcher as the permanent fallback.
- Lazily discover a `wgpu` adapter on the first sufficiently expensive search.
- Prefer Vulkan for its compute performance and fall back to DX12 on supported
  NVIDIA, AMD, or Intel adapters.
- Upload each prepared search plane once and run exact unsigned-integer
  cross-correlation on the GPU.
- Keep score normalization, thresholding, peak selection, native refinement,
  and result ordering in the existing CPU code.
- Use the CPU directly for small searches where GPU setup and readback would
  cost more than the work saved.

## Failure Handling

GPU initialization, allocation, submission, mapping, validation, or device-loss
failures return control to the existing CPU correlation path for that operation.
Once the GPU backend is known to be unusable, later operations avoid repeatedly
paying initialization costs. A successful GPU search that finds no object is a
valid result and does not run the same full search again on the CPU.

## Correctness

The shader accumulates non-negative pixel products as a two-word unsigned
integer, preserving the exact integer correlation sum used by the CPU. Existing
CPU code remains responsible for floating-point normalization and every
detection decision. Tests compare GPU and CPU correlation planes when a
compatible adapter is available and separately prove fallback behavior.

## Performance Validation

Compare the existing release benchmark baselines for present-cold,
present-remembered, and absent full-screen searches. Retain GPU acceleration
only for workloads where it improves elapsed time by at least 10%; remembered
and small-region searches remain on CPU.
