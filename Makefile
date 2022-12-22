.PHONY: deps build start stop clean down

build:
	docker compose build

start:
	docker compose up

stop:
	docker compose stop

down:
	docker compose down

