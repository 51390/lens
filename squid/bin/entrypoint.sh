
LOGFILE=/squid/var/logs/extended.log

touch $LOGFILE
chmod a+w $LOGFILE

sleep 10  # wait for icap server to come online....

/squid/sbin/squid

tail -f $LOGFILE
