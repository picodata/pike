.PHONY: tests
tests:
	cargo test -- --test-threads=1

.PHONY: style-check
style-check:
	cargo check --all --bins --tests --benches
	cargo fmt -- --check --config use_try_shorthand=true
	cargo clippy --all-features --bins --examples --tests --benches -- -W clippy::all -W clippy::pedantic -D warnings
