#!/bin/bash
# Shared setup: LOG is the dump tool's log (tee output), FIFO the fifo feeding its stdin,
# SLOT the wheel's slot. See docs/probes/README.md.
# usage: oracle.sh <file of packets>; sends each and prints those that draw a reply, with the
# reply. Flags steering swings (!!MOVED from 1500). After a reset it waits for the wheel to
# rejoin, sets the LED and motor again (SPRING, default 48) and carries on.
L=${LOG:?set LOG to the dump log}; FIFO=${FIFO:?set FIFO to the dump input fifo}; SLOT=${SLOT:-0}
GAP=${GAP:-0.25}; SPRING=${SPRING:-48}
send() { echo "$SLOT $1" > "$FIFO"; }
setup() { for c in "00 00 08 42" "00 00 0b 04" "00 00 0b $SPRING"; do send "$c 00 00 00 00 00 00 00 00"; sleep 0.4; done; }
while read -r p; do
  [ -z "$p" ] && continue; [[ "$p" == \#* ]] && { echo "$p"; continue; }
  [[ "$(echo $p | cut -d' ' -f1,3,4)" == "00 08 c0" ]] && { echo "REFUSED $p"; continue; }
  n=$(wc -l < $L); send "$p"; sleep $GAP
  out=$(tail -n +$((n+1)) $L)
  # A deliberate reset (00 00 0f 81-bf) drops the wheel about 1 s later: wait for it.
  b=($p); if [[ "${b[1]} ${b[2]}" == "00 0f" ]] && (( 16#${b[3]} >= 0x81 && 16#${b[3]} <= 0xbf )); then
    for i in $(seq 40); do echo "$out" | grep -q 'IN  08 00' && break; sleep 0.25; out=$(tail -n +$((n+1)) $L); done
  fi
  r=$(echo "$out" | grep ' IN ' | grep -v -E '00 01 00 f0|IN  00 00 00 f0|IN  00 f8|IN  08 (00|80)' | sed 's/^ *[0-9.]* slot[0-3] IN  //; s/\( 00\)*$//' | tr '\n' ';')
  sw=$(echo "$out" | awk '/IN  00 01 00/{v=strtonum("0x"$15$14); if(v>32767)v-=65536; if(!s){lo=hi=v;s=1} if(v<lo)lo=v; if(v>hi)hi=v} END{print hi-lo}')
  flag=""; [ "$sw" -ge 1500 ] && flag=" !!MOVED $sw"
  if echo "$out" | grep -q 'IN  08 00'; then
    for i in $(seq 80); do echo "$out" | grep -q 'IN  08 80' && break; sleep 0.25; out=$(tail -n +$((n+1)) $L); done
    sleep 1; setup; flag="$flag RESET"
  fi
  [ -n "$r$flag" ] && echo "$p | $r$flag"
done < "$1"
echo "-- end"
