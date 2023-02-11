
ulimit -n 65536
LOGFILE=/squid/var/logs/extended.log

touch $LOGFILE
chmod a+w $LOGFILE

sleep 10  # wait for icap server to come online....

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


tail -f $LOGFILE
