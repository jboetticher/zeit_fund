docker pull ipfs/go-ipfs:latest
docker run \
   -p 4001:4001 \
   -p 127.0.0.1:8080:8080 \
   -p 127.0.0.1:8081:8081 \
   -p 127.0.0.1:5001:5001 \
   ipfs/go-ipfs