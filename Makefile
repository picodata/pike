.PHONY: tests
tests:
	cargo test -- --nocapture --test-threads=1
