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
RUN ./configure --disable-dependency-tracking --enable-ssl-crtd --with-openssl --disable-strict-error-checking --prefix=$INSTALL_PATH
RUN make
RUN make install

WORKDIR $INSTALL_PATH

ADD conf/squid/* /squid/etc/
RUN chmod 400 /squid/etc/bump*
RUN /squid/libexec/security_file_certgen -c -s /squid/var/cache/squid/ssl_db -M 20MB
RUN chmod a+w -R /squid/var/logs 

ADD bin/entrypoint.sh /squid/bin/entrypoint.sh

ENTRYPOINT bash /squid/bin/entrypoint.sh
