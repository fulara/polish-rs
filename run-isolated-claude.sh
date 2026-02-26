docker build -t claude:isolated-polish-rs .
docker run --rm -it   --user "$(id -u):$(id -g)"   -e HOME=/home/node   -v "$(pwd)":/srv:Z   -v "$(pwd)/claude-home":/home/node:Z   -w /srv   claude:isolated-polish-rs
