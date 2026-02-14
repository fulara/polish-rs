sudo docker build -t claude:isolated .
sudo docker run  --rm -it -v $(pwd):/srv:Z claude:isolated
