.PHONY: deps build start stop clean down

PLATFORM:=$(shell uname)
BIND_SOURCE:=$(shell test $(PLATFORM) = 'Darwin' && echo /tmp || echo /var/lib/systemd/coredump/)
ARCH_FLAGS=$(shell test $(PLATFORM) = 'Darwin' && echo '--build=aarch64-unknown-linux-gnu')

build:
	BIND_SOURCE=$(BIND_SOURCE) \
	docker compose build --build-arg ARCH_FLAGS=$(ARCH_FLAGS)

start:
	BIND_SOURCE=$(BIND_SOURCE) docker compose up

stop:
	BIND_SOURCE=$(BIND_SOURCE) docker compose stop

down:
	BIND_SOURCE=$(BIND_SOURCE) docker compose down

