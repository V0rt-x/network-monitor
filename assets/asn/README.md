# Autonomous system directory

This pair of files is how the application turns an address into a name a person can read —
"Cloudflare", "Amazon", the user's own provider — instead of four numbers that mean nothing
to anyone. It is the data behind endpoint attribution and the naming of hops in the route
panel.

Like every other list in `assets/`, it is **bundled and never fetched**. Nothing here is
downloaded at runtime, at first run, or behind the user's back; a fresher snapshot ships
with a new release of the app.

## Why it is bundled, and not downloaded on demand

This was reconsidered deliberately and the answer came back the same. Downloading the
database at install time is attractive — a smaller installer, a fresher table — and it
fails for precisely the people this application is built for.

Every ready-made IP-to-ASN file on the internet is served from behind Cloudflare or Fastly:
iptoasn, DB-IP, GitHub raw, jsDelivr. Since June 2025 Russian ISPs have throttled
Cloudflare-backed responses to roughly the first 16 KB of any asset, which breaks a
multi-megabyte download completely, and Iranian filtering reaches the same CDNs by other
means. The only sources on their own infrastructure are RIPE NCC and RouteViews, and
neither publishes a lookup table — they publish raw BGP dumps, which are an order of
magnitude larger and need an MRT parser to be useful.

So there is no host that is reliably reachable from the countries this product exists for,
and a host that is reachable today may not be next year. A download would make the feature
work everywhere except where it is needed. It ships in the box.

## Source and licence

| | |
|---|---|
| Upstream | <https://iptoasn.com/data/ip2asn-combined.tsv.gz> |
| Licence | Open Data Commons Public Domain Dedication and Licence (PDDL) v1.0 |
| Attribution required | No |
| Derived from | Regional registry allocations and public BGP routing archives |
| Snapshot retrieved | 2026-08-03 |
| Upstream sha256 | `1697a5115c88f79fd4e00868f909ce11a83719ab97f50f51693771780a144946` |

PDDL places the data in the public domain, so bundling and redistributing it carries no
obligation at all. That is why this source was chosen over the alternatives: DB-IP's Lite
database is CC BY 4.0 and requires a visible link back to db-ip.com on any page showing its
results, and MaxMind's GeoLite2 requires an account and acceptance of an end-user licence,
which is a poor thing to ask of this audience even before its download is considered.

## The two files

The upstream file repeats an AS description on every one of its rows. Splitting the
descriptions into a directory of their own removes that repetition and takes the bundle
from 8.46 MB to 5.11 MB, while leaving both halves as plain tab-separated text that
anyone can read.

```
ranges.tsv.gz   range_start  range_end  as_number          573,125 rows
asn.tsv.gz      as_number    country    as_description      86,628 rows
```

`range_start` and `range_end` are inclusive, written as ordinary IPv4 or IPv6 literals, and
the two families are interleaved in one file. Rows upstream marks as `Not routed`
(AS number 0) are dropped: they describe address space nobody announces, which is a
question this application never asks.

`country` is the registration country of the autonomous system. It is **not** where the
machine that answered is standing, and the user interface must never present it as such —
an anycast address and a cloud region both routinely put the two thousands of kilometres
apart. The measured round trip is the better evidence of distance, never the other way
round.

## Refreshing the snapshot

One command, reproducible, and the output is deterministic given the same upstream file:

```sh
curl -O https://iptoasn.com/data/ip2asn-combined.tsv.gz
gzip -dc ip2asn-combined.tsv.gz > combined.tsv
awk -F'\t' '$3!=0 {print $1"\t"$2"\t"$3}' combined.tsv | gzip -9c > ranges.tsv.gz
awk -F'\t' '$3!=0 && !seen[$3]++ {print $3"\t"$4"\t"$5}' combined.tsv | sort -n | gzip -9c > asn.tsv.gz
```

Update the retrieval date and the upstream checksum in the table above in the same commit.
The parser tolerates rows in any order and rejects a malformed file rather than silently
losing half of it, so a botched regeneration fails loudly at load rather than quietly
mis-naming somebody's provider.
