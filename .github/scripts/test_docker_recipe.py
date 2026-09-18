#!/usr/bin/env python3
"""Check recipe provenance without building images or resolving dependencies."""
from pathlib import Path


root = Path(__file__).resolve().parents[2]
dockerfile = (root / "docker/base-image/Dockerfile").read_text()
assert "COPY recipe.json recipe.json" not in dockerfile, "Cook must not consume the tracked, potentially stale recipe"
assert "FROM chef AS planner" in dockerfile, "Generate the recipe in a separate stage using the same Rust toolchain"
planner, builder = dockerfile.split("FROM chef AS planner", 1)[1].split("FROM chef AS rust-builder", 1)
assert planner.index("COPY . .") < planner.index("RUN cargo chef prepare --recipe-path recipe.json")
generated_recipe = "COPY --from=planner /wildcat/recipe.json recipe.json"
assert builder.index(generated_recipe) < builder.index("chef cook --release") < builder.index("COPY . .")
assert "RUN cargo install cargo-chef" in dockerfile.split("FROM chef AS planner", 1)[0]
assert "WORKDIR /wildcat" in dockerfile.split("FROM chef AS planner", 1)[0]
print("PASS: dependency cook consumes the recipe generated from the current source in the shared planner stage")
