FROM quay.io/podman/stable:v5.6.1

SHELL [ "/bin/bash", "-c" ]
WORKDIR /root

# Copy over the repo (except for the Docker files)
ADD .gitignore README.md LICENSE Cargo.toml diesel.toml /autograder/
ADD diesel    /autograder/diesel
ADD example   /autograder/example
ADD src       /autograder/src
ADD templates /autograder/templates

# Install rust
RUN dnf -y install gcc libpq-devel git file \
    && curl -Lo rustup.sh --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    && chmod +x rustup.sh \
    && ./rustup.sh -y \
    && rm rustup.sh

WORKDIR /autograder

# Build the autograder
ARG CARGO_BUILD_FLAGS
RUN source /root/.bashrc \
    && cargo build ${CARGO_BUILD_FLAGS} \
    && cargo install diesel_cli --no-default-features --features "postgres"

ENV PATH="/root/.cargo/bin:$PATH"

CMD [ "bash" ]
