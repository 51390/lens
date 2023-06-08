
ulimit -n 65536
LOGFILE=/squid/var/logs/extended.log

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
    /squid/sbin/squid
fi;


sleep 5
export KID_PID=`ps -C squid -o pid,args | grep kid | awk '{ print $1 }'`
echo "Will attach perf to pid $KID_PID"


perf record --pid=$KID_PID -o /tmp/perf.data &

tail -f $LOGFILE
