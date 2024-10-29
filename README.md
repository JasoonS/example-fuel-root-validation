# Invalid receipt validation example

This repo contains a reproduction of the issue of validating transaction receipts on Fuel Testnet in the 3.6 million block range.

Run the code with `cargo run`

All the blocks in the `block_heights` vector contain receipt_roots that are deemed invalid by the root generation function.
