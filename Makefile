.PHONY: squid

build:
	docker compose build

start:
	docker compose up

stop:
	docker compose stop

clean:
	docker compose down

