# clear possibly dangling pid files
rm /squid/var/run/squid.pid

ulimit -n 65536
ulimit -c unlimited

LOGFILE=/squid/var/logs/extended.log
#PERFTOOLS=${PERFTOOLS:-1}

echo "/tmp/coredump/core.%t.%e.%p" > /proc/sys/kernel/core_pattern
    
rsyslogd

touch $LOGFILE
chmod a+w $LOGFILE

if [ $VALGRIND == 1 ]
then
    valgrind -v \
        --trace-children=yes \
        --num-callers=50 \
        --xml=yes \
        --xml-file=/tmp/valgrind-%p.xml \
        --log-file=/tmp/valgrind-%p.log \
        --leak-check=full \
        --leak-resolution=high \
        --show-reachable=yes \
        /squid/sbin/squid -N &
else
    /squid/sbin/squid -N &
fi;

#tail -f $LOGFILE
tail --retry -f /var/log/syslog
