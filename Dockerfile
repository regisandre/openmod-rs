# Build: docker build -t openmod .
# Run:   docker run -d --name openmod -v openmod-data:/data \
#          -p 8088:8088 -p 127.0.0.1:8089:8089 openmod
#        docker logs openmod   (the admin token, at first start)
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock openmod.toml ./
COPY .cargo .cargo
COPY src src
COPY ui ui
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends libstdc++6 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data openmod \
 && mkdir /data && chown openmod /data
COPY --from=build /src/target/release/openmod /usr/local/bin/openmod
USER openmod
WORKDIR /data
VOLUME /data
# The configuration, the models and the state file live in /data; the first
# start writes a default configuration there. Both listeners take every
# interface inside the container: publish the admin port on the host's
# loopback only (-p 127.0.0.1:8089:8089).
ENV OPENMOD_CONFIG=/data/openmod.toml \
    OPENMOD_BIND=0.0.0.0:8088 \
    OPENMOD_ADMIN_BIND=0.0.0.0:8089
EXPOSE 8088 8089
ENTRYPOINT ["openmod"]
CMD ["serve"]
