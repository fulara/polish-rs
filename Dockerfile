FROM node:current-alpine

RUN apk add --no-cache \
    curl \
    ca-certificates \
    bash \
    git \
    build-base \
    pkgconf \
    openssl-dev

# Install rustup + stable toolchain
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain stable \
 && rustc --version \
 && cargo --version

RUN npm install -g @anthropic-ai/claude-code

WORKDIR /srv

CMD ["claude"]

