docker run \
    -p 30333:30333 \
    -p 9933:9933 \
    -p 9944:9944 \
    --name=zeitgeist-parachain \
    --restart=always \
    zeitgeistpm/zeitgeist-node:latest \ 
    --base-path=/zeitgeist/data \
    --chain=battery_station \
    --name=zeitgeist-support-$RANDOM \
    --pruning=archive