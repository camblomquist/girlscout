FROM rust:1-alpine3.19 as build

WORKDIR /girlscout
COPY . .

RUN apk add --no-cache clang
RUN cargo install --path .

FROM alpine:3.19

COPY --from=build /usr/local/cargo/bin/girlscout /usr/local/bin/girlscout

ENTRYPOINT [ "girlscout" ]
