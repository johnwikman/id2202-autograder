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
ADD container /autograder/container
ADD diesel    /autograder/diesel
ADD example   /autograder/example
ADD macros    /autograder/macros
ADD src       /autograder/src
ADD web       /autograder/web

WORKDIR /autograder

# Build the autograder
ARG CARGO_BUILD_FLAGS
RUN source /root/.bashrc \
    && cargo build ${CARGO_BUILD_FLAGS} \
    && cargo install diesel_cli --no-default-features --features "postgres"

# The base image is configured for rootless podman, which has to go through
# fuse-overlayfs. Here podman runs as root on a volume that supports overlayfs
# natively, so let it use the kernel driver: binaries cannot be executed from a
# FUSE mount on some kernels (notably the one Docker Desktop ships), which
# otherwise fails every grading container with "exec container process:
# Invalid argument". `fsync=0` goes along with it, as the kernel driver
# rejects that fuse-overlayfs option with EINVAL when mounting.
RUN sed -i -e '/^\s*mount_program\s*=/d' \
           -e 's/^\s*mountopt\s*=.*/mountopt = "nodev"/' \
           /etc/containers/storage.conf

ENV PATH="/root/.cargo/bin:$PATH"

CMD [ "bash" ]
