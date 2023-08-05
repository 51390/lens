FROM ubuntu

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
    libssl-dev libcppunit-dev make pkg-config build-essential

# arch flags
ARG BUILD_ROOT=/build
ARG ARCH_SCRIPT=$BUILD_ROOT/arch.sh
RUN mkdir $BUILD_ROOT
RUN echo 'test $(uname -i) = "aarch64" && echo -n "--build=aarch64-unknown-linux-gnu"' > $ARCH_SCRIPT

# libecap
ARG ECAP_BUILD_PATH=$BUILD_ROOT/ecap
RUN mkdir $ECAP_BUILD_PATH
WORKDIR $ECAP_BUILD_PATH
RUN wget https://www.e-cap.org/archive/libecap-1.0.0.tar.gz
RUN tar xvpfz libecap-1.0.0.tar.gz
WORKDIR $ECAP_BUILD_PATH/libecap-1.0.0
RUN ./configure `bash $ARCH_SCRIPT`  && make && make install
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

FROM prism
FROM ecap-stream
COPY --from=0 /squid /squid
COPY --from=1 /prism /prism

RUN apt update -y --fix-missing; \
    apt install -y \
    vim gdb zstd zlib1g-dev valgrind rsyslog smem \
    curl wget

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
