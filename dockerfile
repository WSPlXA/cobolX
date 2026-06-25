# syntax=docker/dockerfile:1

FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        pkg-config \
        libssl-dev \
        gdb \
        lldb \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /workspaces/cobolX/target \
    && chown -R vscode:vscode /workspaces

WORKDIR /workspaces/cobolX
