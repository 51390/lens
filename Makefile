.PHONY: build start stop down


PLATFORM:=$(shell uname)
BIND_SOURCE:=$(shell test $(PLATFORM) = 'Darwin' && echo /tmp || echo /var/lib/systemd/coredump/)
ARCH_FLAGS:=$(shell test $(PLATFORM) = 'Darwin' && echo '--build=aarch64-unknown-linux-gnu')
VALGRIND ?= 0
VALGRIND_FLAGS:=$(shell test $(VALGRIND) == 1 && echo '"--with-valgrind-debug --disable-optimizations"')

build: build-ecap-stream build-prism
	COMPOSE_DOCKER_CLI_BUILD=1 \
	DOCKER_BUILDKIT=1 \
	BIND_SOURCE=$(BIND_SOURCE) \
	docker compose build \
		--build-arg ARCH_FLAGS=$(ARCH_FLAGS) \
		--build-arg VALGRIND_FLAGS=$(VALGRIND_FLAGS) \
		--build-arg VALGRIND=$(VALGRIND)

build-ecap-stream:
	git submodule update --init
	docker build -t ecap-stream ecap-stream

build-prism:
	git submodule update --init
	docker build -t prism prism

start:
	BIND_SOURCE=$(BIND_SOURCE) docker compose up

stop:
	BIND_SOURCE=$(BIND_SOURCE) docker compose stop

down:
	BIND_SOURCE=$(BIND_SOURCE) docker compose down

