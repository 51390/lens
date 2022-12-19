
LOGFILE=/squid/var/logs/extended.log

touch $LOGFILE
chmod a+w $LOGFILE

/squid/sbin/squid

tail -f $LOGFILE
