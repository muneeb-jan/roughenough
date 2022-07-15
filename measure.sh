#!/bin/bash
echo "Calling_Time,Time_Received_From_RoughEnough,Receive_Time" > log.csv
for iter in {1..50000}
do
#tn=`date +'%T.%3N'`
#ms=`echo $tn | awk '{split($0,a,"."); print a[2]}'`
#while [ "$ms" -le 918 ] || [ "$ms" -ge 920 ]
#while [ "$ms" -ne 890 ]
#do
#tn=`date +'%T.%3N'`
#ms=`echo $tn | awk '{split($0,a,"."); print a[2]}'`
#done
#echo "`date +'%T.%3N'`   Calling roughtime.int08h.com trial number $iter" >> log.txt
#echo "`date +'%T.%3N'`,`target/release/roughenough-client -v roughtime.int08h.com 2002 -k "AW5uAoTSTDfG5NfY1bTh08GUnOqlRb+HVhbJ3ODJvsE=" | awk '{print $4}'`,`date +'%T.%3N'`" >> log.csv
echo "`date +'%T.%3N'`,`target/release/roughenough-client -v roughtime.int08h.com 2002 | awk '{print $4}'`,`date +'%T.%3N'`" >> log.csv
done
