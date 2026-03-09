.PHONY: build build-api build-operator docker-api docker-operator deploy test test-junit fmt clippy install-tools

REGISTRY ?= ghcr.io/nearai
TAG ?= latest

build: build-api build-operator

build-api:
	cargo build --release -p agent-api

build-operator:
	cargo build --release -p agent-operator

docker-api:
	docker build -f Dockerfile.api -t $(REGISTRY)/agent-api:$(TAG) .

docker-operator:
	docker build -f Dockerfile.operator -t $(REGISTRY)/agent-operator:$(TAG) .

docker: docker-api docker-operator

push: docker
	docker push $(REGISTRY)/agent-api:$(TAG)
	docker push $(REGISTRY)/agent-operator:$(TAG)

deploy:
	bash scripts/deploy.sh

test:
	cargo test --workspace

test-junit:
	cargo nextest run --workspace --profile ci

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace -- -D warnings

install-tools:
	cargo install cargo-nextest

crd-gen:
	cargo run -p agent-operator -- --crd-gen > deploy/manifests/agent-crd.yaml
