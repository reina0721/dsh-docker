.PHONY: build fmt lint test check image image-slim deploy up down stop status doctor backup restore shell completions

CARGO ?= cargo

build:
	$(CARGO) build --workspace --release --locked

fmt:
	$(CARGO) fmt --all

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

test:
	$(CARGO) test --workspace --locked

check: fmt lint test
	$(CARGO) fmt --all -- --check
	docker compose config >/dev/null

image:
	docker build --target dev -t dsh-docker:dev .

image-slim:
	docker build --target runtime -t dsh-docker:slim .

deploy:
	$(CARGO) run --release -p dshctl -- deploy

up:
	$(CARGO) run --release -p dshctl -- up

down:
	$(CARGO) run --release -p dshctl -- down

stop:
	$(CARGO) run --release -p dshctl -- stop

status:
	$(CARGO) run --release -p dshctl -- status

doctor:
	$(CARGO) run --release -p dshctl -- doctor

backup:
	$(CARGO) run --release -p dshctl -- backup

restore:
	@test -n "$(ARCHIVE)" || (echo "usage: make restore ARCHIVE=path/to/backup.tar.gz" >&2; exit 1)
	$(CARGO) run --release -p dshctl -- restore "$(ARCHIVE)" --yes

shell:
	docker compose exec dsh bash

completions:
	$(CARGO) run --release -p dshctl -- completion bash > dshctl.bash
	$(CARGO) run --release -p dshctl -- completion zsh > _dshctl
	$(CARGO) run --release -p dshctl -- completion fish > dshctl.fish
