CARGO ?= cargo
BIN    ?= ai-sandbox-landlock

.PHONY: all build run debug test fmt clippy clean install

all: build

build:
	$(CARGO) build --release --bin $(BIN)

debug: build

run:
	@if [ -z "$(ROOT)" ]; then \
		echo "Usage: make run ROOT=/path/to/project CMD='code .'"; \
		exit 1; \
	fi
	@if [ -z "$(CMD)" ]; then \
		echo "Error: CMD variable is empty (example: CMD='code .')"; \
		exit 1; \
	fi
	@$(CARGO) run --bin $(BIN) -- --root "$(ROOT)" -- $$CMD

test:
	@$(CARGO) test

fmt:
	@$(CARGO) fmt

clippy:
	@$(CARGO) clippy --all-targets -- -D warnings

clean:
	@$(CARGO) clean

install:
	@mkdir -p $(HOME)/.local/bin
	install -Dm755 target/release/$(BIN) $(HOME)/.local/bin/$(BIN)
	@echo "Installed $(BIN) to $(HOME)/.local/bin"
