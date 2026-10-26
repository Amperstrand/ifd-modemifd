# Bench UFW unblock recipe — router internet via the bench host (issue #45)

**Status: RECIPE ONLY — owner-executed (needs sudo on the bench host).**
Verified topology 2026-10-26; commands NOT yet executed (the owner gates
firewall changes — that is the whole point of #45).

## Topology (measured)

```
NR7101 lan.200 (10.200.0.2/24) ── VLAN 200 ── bench enx.200 (10.200.0.1/24)
bench wlp4s0 (192.168.13.221, default via 192.168.13.1) ── internet
```

The router already carries `default via 10.200.0.1 dev lan.200 metric 5`
(fallback — primary default is the modem's `wwan0`). The bench host is the
intended NAT gateway for that fallback; today it drops the traffic (UFW).

## Recipe (bench host, sudo)

```sh
# 1. forwarding
sudo sysctl -w net.ipv4.ip_forward=1
echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-forward.conf

# 2. UFW: allow the router's VLAN in, and the forwarded range out
sudo ufw allow in on enx.200 from 10.200.0.0/24
sudo ufw route allow in on enx.200 from 10.200.0.0/24 to any

# 3. NAT the router traffic out the bench uplink
#    (UFW's before.rules keeps this across reloads)
sudo iptables -t nat -A POSTROUTING -s 10.200.0.0/24 -o wlp4s0 -j MASQUERADE
# persistent variant: add the same rule to /etc/ufw/before.rules *nat section

# 4. verify from the router
ssh root@192.168.1.10 'ping -c3 -I lan.200 9.9.9.9'
```

## Why this exists

The on-router provisioning does NOT need this path (the download rides the
modem itself — see AGENTS.md "Where the provisioning data flows"). This
recipe matters for: firmware downloads for #38 (large files over cellular
are expensive/flaky), OpenWrt package installs without offline bundles, and
any recovery scenario where the modem data path is down but profiles must
be managed.

Caveats: bench host is WiFi-attached (`wlp4s0`); if the bench's uplink
changes (cable dock), the MASQUERADE `-o` interface must follow. Docker is
present on the bench — its iptables chains coexist with UFW's; test after
any docker daemon upgrade.
