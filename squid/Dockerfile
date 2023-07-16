# syntax = docker/dockerfile:1.2

FROM ecap-stream

ARG ARCH_FLAGS
ARG VALGRIND
ARG VALGRIND_FLAGS
ARG CARGO_HOME

ENV VALGRIND=${VALGRIND}
ENV CARGO_HOME=${CARGO_HOME}

# tz
ENV TZ=Etc/UTC
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone

# package dependencies
RUN apt update -y --fix-missing; \
    apt install -y \
    curl wget gcc g++ automake autoconf libtool openssl \
    libssl-dev libcppunit-dev make pkg-config vim gdb zstd \
    zlib1g-dev valgrind rsyslog build-essential \
    google-perftools libgoogle-perftools-dev smem

# link in gperftools
ENV LIBS=-ltcmalloc_and_profiler
ENV CFLAGS=-ltcmalloc_and_profiler
ENV CXXFLAGS=-ltcmalloc_and_profiler

# rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
RUN bash /tmp/rustup.sh -y
RUN bash -c 'PATH="$HOME/.cargo/bin:$PATH" rustup toolchain install stable'
RUN bash -c 'PATH="$HOME/.cargo/bin:$PATH" rustup default stable'

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

# analyzer
ARG ANALYZER_BUILD_PATH=/tmp/analyzer/
ADD analyzer $ANALYZER_BUILD_PATH
WORKDIR $ANALYZER_BUILD_PATH
RUN PATH=$PATH:${CARGO_HOME}/bin make build

# squid config
ADD conf/* /squid/etc/
RUN chmod 400 /squid/etc/bump*
RUN /squid/libexec/security_file_certgen -c -s /squid/var/cache/squid/ssl_db -M 20MB
RUN chmod a+w -R /squid/

# syslog
RUN sed -i '/imklog/s/^/#/' /etc/rsyslog.conf

# gperftools heap profiler configuration
#ENV HEAPPROFILE=/tmp/heap.prof
#ENV HEAPPROFILESIGNAL=28

# gperftools heap checker configuration; note: conflicts with the heap profiler
#ENV HEAPCHECK=normal

WORKDIR $INSTALL_PATH
ADD bin/entrypoint.sh /squid/bin/entrypoint.sh
ENTRYPOINT bash /squid/bin/entrypoint.sh
