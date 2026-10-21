.PHONY: check fmt lint test cov

check: fmt lint test

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace --all-features

cov:
	cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines $${BROSKI_COVERAGE_MIN:-50}
