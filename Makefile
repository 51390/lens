.PHONY: build start stop down


PLATFORM:=$(shell uname)
BIND_SOURCE:=$(shell test $(PLATFORM) = 'Darwin' && echo /tmp || echo /var/lib/systemd/coredump/)
ARCH_FLAGS:=$(shell test $(PLATFORM) = 'Darwin' && echo '--build=aarch64-unknown-linux-gnu')
VALGRIND ?= 0
VALGRIND_FLAGS:=$(shell test $(VALGRIND) == 1 && echo '"--with-valgrind-debug --disable-optimizations"')

ifndef PROJECT_NAME
PROJECT_NAME:=lens
endif

build: build-ecap-stream build-prism build-elastic-stack
	COMPOSE_DOCKER_CLI_BUILD=1 \
	DOCKER_BUILDKIT=1 \
	BIND_SOURCE=$(BIND_SOURCE) \
	docker compose build \
		--build-arg ARCH_FLAGS=$(ARCH_FLAGS) \
		--build-arg VALGRIND_FLAGS=$(VALGRIND_FLAGS) \
		--build-arg VALGRIND=$(VALGRIND)

submodules:
	git submodule update --init

build-ecap-stream: submodules
	docker build -t ecap-stream ecap-stream

build-prism: submodules
	docker build -t prism prism

build-elastic-stack: submodules
	PROJECT_NAME=$(PROJECT_NAME) make -C elastic-stack build

start:
	BIND_SOURCE=$(BIND_SOURCE) docker compose up

start-stack:
	# starts the proxy along with an 
	# elasticsearch/logstash/kibana stack
	# to be used as a backend
	BIND_SOURCE=$(BIND_SOURCE) docker compose -f ./elastic-stack/stack/docker-compose.yml -f ./docker-compose.yml -p $(PROJECT_NAME) up

down-stack:
	BIND_SOURCE=$(BIND_SOURCE) docker compose -f ./elastic-stack/stack/docker-compose.yml -f ./docker-compose.yml -p $(PROJECT_NAME) down

stop:
	BIND_SOURCE=$(BIND_SOURCE) docker compose stop

down:
	BIND_SOURCE=$(BIND_SOURCE) docker compose down

extract-certificate:
	docker cp lens-proxy-1:/squid/etc/bump.crt /tmp/
