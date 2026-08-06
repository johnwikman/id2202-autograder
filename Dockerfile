FROM quay.io/podman/stable:v5.8.2

SHELL [ "/bin/bash", "-c" ]
WORKDIR /root

# Install rust
RUN dnf -y install gcc libpq-devel git file \
    && curl -Lo rustup.sh --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    && chmod +x rustup.sh \
    && ./rustup.sh -y \
    && rm rustup.sh

# Copy over the repo (except for the Docker files)
ADD .gitignore README.md LICENSE Cargo.toml Cargo.lock build.rs diesel.toml sailfish.toml /autograder/
ADD diesel    /autograder/diesel
ADD example   /autograder/example
ADD src       /autograder/src
ADD web       /autograder/web

WORKDIR /autograder

# Build the autograder
ARG CARGO_BUILD_FLAGS
RUN source /root/.bashrc \
    && cargo build ${CARGO_BUILD_FLAGS} \
    && cargo install diesel_cli --no-default-features --features "postgres"

ENV PATH="/root/.cargo/bin:$PATH"

CMD [ "bash" ]
