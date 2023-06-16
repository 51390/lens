
ulimit -n 65536
LOGFILE=/squid/var/logs/extended.log
PERFTOOLS=${PERFTOOLS:-1}
    
rsyslogd

touch $LOGFILE
chmod a+w $LOGFILE

if [ $VALGRIND == 1 ]
then
    valgrind -v \
        --trace-children=yes \
        --num-callers=50 \
        --log-file=/tmp/valgrind-%p.log \
        --leak-check=full \
        --leak-resolution=high \
        --show-reachable=yes \
        /squid/sbin/squid
else
    /squid/sbin/squid -N &
fi;

#tail -f $LOGFILE
tail --retry -f /var/log/syslog
