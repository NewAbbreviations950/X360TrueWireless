#!/bin/bash
# Shared setup: LOG is the dump tool's log (tee output), FIFO the fifo feeding its stdin,
# SLOT the wheel's slot. See docs/probes/README.md.
# usage: each.sh <cmd bytes 2-3 ...>; sends each command alone, then the motor off/on check.
L=${LOG:?set LOG to the dump log}; FIFO=${FIFO:?set FIFO to the dump input fifo}; SLOT=${SLOT:-0}
send() { echo "$SLOT $1 00 00 00 00 00 00 00 00" > "$FIFO"; }
for cmd in "$@"; do
  n=$(wc -l < $L); send "00 00 ${cmd/:/ }"; sleep 0.3
  send "00 00 0b 02"; sleep 1.5; send "00 00 0b 04"; sleep 0.3; send "00 00 0b 5f"; sleep 1.5
  r=$(tail -n +$((n+1)) $L | awk '/IN  00 01 00 f0/{v=strtonum("0x"$15$14); if(v>32767)v-=65536; if(!s){lo=hi=v;s=1} if(v<lo)lo=v; if(v>hi)hi=v; c++} END{print c+0, "reports, swing", hi-lo}')
  echo "${cmd/:/ }: $r"
done
