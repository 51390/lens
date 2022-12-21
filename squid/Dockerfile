FROM ubuntu

ARG BUILD_PATH=/tmp/build
ARG INSTALL_PATH=/squid


RUN mkdir $BUILD_PATH
WORKDIR $BUILD_PATH

RUN apt update -y
RUN apt install -y  wget gcc g++ automake autoconf libtool openssl libssl-dev libcppunit-dev make
RUN wget http://www.squid-cache.org/Versions/v5/squid-5.7.tar.gz
RUN tar xvpfz squid-5.7.tar.gz

WORKDIR $BUILD_PATH/squid-5.7

RUN bash bootstrap.sh
RUN ./configure --disable-dependency-tracking --enable-ssl-crtd --with-openssl --disable-strict-error-checking --enable-ecap --prefix=$INSTALL_PATH
RUN make
RUN make install

RUN mkdir /tmp/build-ecap
WORKDIR /tmp/build-ecap
RUN wget https://www.e-cap.org/archive/libecap-1.0.0.tar.gz
RUN tar xvpfz libecap-1.0.0.tar.gz

RUN apt install -y pkg-config

WORKDIR /tmp/build-ecap/libecap-1.0.0
RUN ./configure && make && make install

RUN mkdir /tmp/build-ecap-adapter
WORKDIR /tmp/build-ecap-adapter
RUN wget https://www.e-cap.org/archive/ecap_adapter_sample-1.0.0.tar.gz
RUN tar xvfpz ecap_adapter_sample-1.0.0.tar.gz
WORKDIR ecap_adapter_sample-1.0.0
COPY ./patch/adapter_async.cc src/adapter_async.cc

RUN ./configure && make && make install

RUN ldconfig

WORKDIR $INSTALL_PATH

ADD conf/* /squid/etc/
RUN chmod 400 /squid/etc/bump*
RUN /squid/libexec/security_file_certgen -c -s /squid/var/cache/squid/ssl_db -M 20MB
RUN chmod a+w -R /squid/var/logs 

ADD bin/entrypoint.sh /squid/bin/entrypoint.sh

ENTRYPOINT bash /squid/bin/entrypoint.sh
