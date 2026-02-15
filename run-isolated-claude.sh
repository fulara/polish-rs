docker build -t claude:isolated .
docker run --rm -it   --user "$(id -u):$(id -g)"   -e HOME=/home/node   -v "$(pwd)":/srv:Z   -v "$(pwd)/claude-home":/home/node:Z   -w /srv   claude:isolated
