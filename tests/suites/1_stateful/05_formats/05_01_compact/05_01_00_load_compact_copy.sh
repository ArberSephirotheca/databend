#!/usr/bin/env bash

CURDIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$CURDIR"/../../../../shell_env.sh

# prepare data
DATA="/tmp/load_compact.csv"
rm -rf $DATA
for j in $(seq 1 1000);do
	printf "0123456789\n" >> "$DATA"
done

echo "drop table if exists t1 all" | $MYSQL_CLIENT_CONNECT
echo "CREATE TABLE t1
(
    c0 string
);" | $MYSQL_CLIENT_CONNECT

echo "---s3 cp"
aws --endpoint-url http://127.0.0.1:9900/ s3 cp $DATA s3://testbucket/$DATA > /dev/null 2>&1

echo "---copy into"
# let input data dispatch to multi threads
echo "set global input_read_buffer_size = 100" | $MYSQL_CLIENT_CONNECT
echo "copy into t1 from 's3://testbucket/${DATA}' connection=(aws_key_id='minioadmin' aws_secret_key='minioadmin' endpoint_url='http://127.0.0.1:9900/') FILE_FORMAT = (type = CSV) force=true" | $MYSQL_CLIENT_CONNECT
echo "set global input_read_buffer_size = 1048576" | $MYSQL_CLIENT_CONNECT

echo "---row_count"
echo "select count(*) from t1" | $MYSQL_CLIENT_CONNECT

echo "---block_count"
echo "select block_count from fuse_snapshot('default','t1')" | $MYSQL_CLIENT_CONNECT

echo "drop table if exists t1" | $MYSQL_CLIENT_CONNECT
