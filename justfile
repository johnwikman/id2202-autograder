image_version := "2.0.0-dev1"
image_name    := "localhost/id2202-autograder"
image_tag     := image_name + ":" + image_version

build-image:
    docker build \
        -t {{image_tag}} \
        --build-arg "CARGO_BUILD_FLAGS=--release" \
        .

rm-image:
    docker rmi {{image_tag}}

setup-dirs:
    mkdir -p data/containers data/log data/postgres data/ssh
