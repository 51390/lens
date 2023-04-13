# syntax = docker/dockerfile:1.2

FROM ubuntu

ARG ARCH_FLAGS
ARG VALGRIND
ARG VALGRIND_FLAGS
ARG CARGO_HOME=/tmp/cargo/

ENV VALGRIND=${VALGRIND}
ENV CARGO_HOME=${CARGO_HOME}

# tz
ENV TZ=Etc/UTC
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone

# package dependencies
RUN apt update -y --fix-missing
RUN apt install -y curl wget gcc g++ automake autoconf libtool openssl libssl-dev libcppunit-dev make pkg-config vim gdb zstd zlib1g-dev valgrind

# rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME},rw bash /tmp/rustup.sh -y

# libecap
ARG ECAP_BUILD_PATH=/tmp/build-ecap
RUN mkdir $ECAP_BUILD_PATH
WORKDIR $ECAP_BUILD_PATH
RUN wget https://www.e-cap.org/archive/libecap-1.0.0.tar.gz
RUN tar xvpfz libecap-1.0.0.tar.gz
WORKDIR $ECAP_BUILD_PATH/libecap-1.0.0
RUN ./configure ${ARCH_FLAGS} && make && make install
RUN ldconfig

# squid
ARG BUILD_PATH=/tmp/build
ARG INSTALL_PATH=/squid
RUN mkdir $BUILD_PATH
WORKDIR $BUILD_PATH
RUN wget http://www.squid-cache.org/Versions/v5/squid-5.7.tar.gz
RUN tar xvpfz squid-5.7.tar.gz

WORKDIR $BUILD_PATH/squid-5.7

RUN bash bootstrap.sh
RUN ./configure ${VALGRIND_FLAGS} --disable-dependency-tracking --enable-ssl-crtd --with-openssl --disable-strict-error-checking --enable-ecap --prefix=$INSTALL_PATH $ARCH_FLAGS
RUN make
RUN make install

# prism
ARG PRISM_BUILD_PATH=/tmp/prism
ADD prism $PRISM_BUILD_PATH
WORKDIR $PRISM_BUILD_PATH
RUN bash bootstrap.sh
RUN ./configure
RUN make
RUN make install
RUN ldconfig

# analyzer
ARG ANALYZER_BUILD_PATH=/tmp/analyzer/
ADD analyzer $ANALYZER_BUILD_PATH
WORKDIR $ANALYZER_BUILD_PATH
RUN echo 8
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME}/registry,sharing=locked,rw ls -alh ${CARGO_HOME}; sleep 10
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME}/registry,sharing=locked,rw ls -alh ${CARGO_HOME}/registry; sleep 10
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME}/registry,sharing=locked,rw ls -alh ${CARGO_HOME}; PATH=$PATH:${CARGO_HOME}/bin make build
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME}/registry,sharing=locked,rw ls -alh ${CARGO_HOME}; sleep 10
RUN --mount=mode=0777,type=cache,target=${CARGO_HOME}/registry,sharing=locked,rw ls -alh ${CARGO_HOME}/registry; sleep 10

# squid config
ADD conf/* /squid/etc/
RUN chmod 400 /squid/etc/bump*
RUN /squid/libexec/security_file_certgen -c -s /squid/var/cache/squid/ssl_db -M 20MB
RUN chmod a+w -R /squid/

WORKDIR $INSTALL_PATH
ADD bin/entrypoint.sh /squid/bin/entrypoint.sh
ENTRYPOINT bash /squid/bin/entrypoint.sh
