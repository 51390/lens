FROM ubuntu

ENV TZ=Etc/UTC
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone

RUN apt update -y
RUN apt install -y  wget gcc g++ automake autoconf libtool openssl libssl-dev libcppunit-dev make pkg-config

# libecap
ARG ECAP_BUILD_PATH=/tmp/build-ecap
RUN mkdir $ECAP_BUILD_PATH
WORKDIR $ECAP_BUILD_PATH
RUN wget https://www.e-cap.org/archive/libecap-1.0.0.tar.gz
RUN tar xvpfz libecap-1.0.0.tar.gz
WORKDIR $ECAP_BUILD_PATH/libecap-1.0.0
RUN ./configure && make && make install
RUN ldconfig

# squid, proper
ARG BUILD_PATH=/tmp/build
ARG INSTALL_PATH=/squid
RUN mkdir $BUILD_PATH
WORKDIR $BUILD_PATH
RUN wget http://www.squid-cache.org/Versions/v5/squid-5.7.tar.gz
RUN tar xvpfz squid-5.7.tar.gz

WORKDIR $BUILD_PATH/squid-5.7

RUN bash bootstrap.sh
RUN ./configure --disable-dependency-tracking --enable-ssl-crtd --with-openssl --disable-strict-error-checking --enable-ecap --prefix=$INSTALL_PATH
RUN make
RUN make install

# sample ecap adapter
ARG ECAP_ADAPTER_BUILD_PATH=/tmp/build-ecap-adapter
RUN mkdir $ECAP_ADAPTER_BUILD_PATH
WORKDIR $ECAP_ADAPTER_BUILD_PATH
RUN wget https://www.e-cap.org/archive/ecap_adapter_sample-1.0.0.tar.gz
RUN tar xvfpz ecap_adapter_sample-1.0.0.tar.gz
WORKDIR ecap_adapter_sample-1.0.0
COPY ./patch/adapter_async.cc src/adapter_async.cc
COPY ./patch/adapter_modifying.cc src/adapter_modifying.cc
RUN CXXFLAGS=-DHAVE_PTHREAD ./configure && make && make install
RUN ldconfig

WORKDIR $INSTALL_PATH

# squid config
ADD conf/* /squid/etc/
RUN chmod 400 /squid/etc/bump*
RUN /squid/libexec/security_file_certgen -c -s /squid/var/cache/squid/ssl_db -M 20MB
RUN chmod a+w -R /squid/

ADD bin/entrypoint.sh /squid/bin/entrypoint.sh

ENTRYPOINT bash /squid/bin/entrypoint.sh
