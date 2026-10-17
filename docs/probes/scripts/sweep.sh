#!/bin/bash
# Shared setup: LOG is the dump tool's log (tee output), FIFO the fifo feeding its stdin,
# SLOT the wheel's slot. See docs/probes/README.md.
# usage: sweep.sh <file>; each line is one step: packets separated by " + " (sent 0.15 s
# apart). After each step: reads reg 9+10, reports resets, steering swing and unusual replies,
# and after a reset waits for the rejoin and restores LED, motor and spring.
L=${LOG:?set LOG to the dump log}; FIFO=${FIFO:?set FIFO to the dump input fifo}; SLOT=${SLOT:-0}
SPRING=${SPRING:-48}; DWELL=${DWELL:-1.2}
send() { local p="$1"; if [[ "$(echo $p | cut -d" " -f1,3,4)" == "00 08 c0" ]]; then echo "REFUSED power off: $p" >&2; return; fi; local nb=$(echo $p | wc -w); while [ $nb -lt 12 ]; do p="$p 00"; nb=$((nb+1)); done; echo "$SLOT $p" > "$FIFO"; }
setup() { send "00 00 08 42"; sleep 0.4; send "00 00 0b 04"; sleep 0.3; send "00 00 0b $SPRING"; sleep 0.8; }
while IFS= read -r line; do
  [ -z "$line" ] && continue; [[ "$line" == \#* ]] && continue
  n=$(wc -l < $L)
  IFS='+' read -ra pk <<< "$line"; for p in "${pk[@]}"; do send "$(echo $p)"; sleep ${GAP:-0.15}; done
  sleep $DWELL
  send "00 00 02 40"; sleep 0.3; send "00 00 02 80"; sleep 0.4
  out=$(tail -n +$((n+1)) $L)
  steer=$(echo "$out" | awk '/IN  00 01 00 f0/{v=strtonum("0x"$15$14); if(v>32767)v-=65536; if(!s){lo=hi=v;s=1} if(v<lo)lo=v; if(v>hi)hi=v} END{print hi-lo}')
  other=$(echo "$out" | grep ' IN ' | grep -v -E '00 01 00 f0|IN  00 00 00 f0|IN  08 (00|80)|00 05 00 f0 00 11 02 00 00 00 00 00 00 00 00 00 00 00|00 05 00 f0 00 12 3f ff ff ff ff ff 00 00 00 00 00 00 ff ff 07 01' | sed 's/^ *[0-9.]* slot[0-3] IN  //; s/\( 00\)*$//' | sort | uniq -c | sed 's/^ *//' | tr '\n' ';')
  flag=""; [ "$steer" -ge 1500 ] && flag=" !!MOVED"
  if echo "$out" | grep -q 'IN  08 00'; then
    flag="$flag RESET"
    for i in $(seq 60); do echo "$out" | grep -q 'IN  08 80' && break; sleep 0.25; out=$(tail -n +$((n+1)) $L); done
    sleep 1; setup
  fi
  echo "$line | steer $steer$flag | $other"
done < "$1"
