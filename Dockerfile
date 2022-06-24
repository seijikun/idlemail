FROM rustlang/rust:nightly as builder

# Create base image only with our dependencies and build once
# This creates a cache with the built dependencies
RUN USER=root cargo new --bin idlemail
WORKDIR ./idlemail
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release
RUN rm src/*.rs

# Add idlemail source-code
ADD . ./

RUN rm ./target/release/deps/idlemail*
RUN cargo build --release

# Build actual runtime container
FROM ubuntu:20.04
ARG APP=/idlemail

RUN apt-get update \
    && apt-get install -y ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

ENV APP_USER=idlemail

RUN groupadd $APP_USER \
    && useradd -g $APP_USER $APP_USER \
    && mkdir -p ${APP}

COPY --from=builder /idlemail/target/release/idlemail ${APP}/idlemail

RUN chown -R $APP_USER:$APP_USER ${APP}

USER $APP_USER
WORKDIR ${APP}

CMD ["./idlemail", "/idlemail/config.json"]
