# Baseline target lists

These files are the *only* places the app learns what to probe for its general network
health view. They are plain JSON, versioned in this repository, and readable by anyone who
wants to audit exactly which addresses the application will contact. Nothing here is
fetched at runtime and nothing is updated behind the user's back: a new list ships with a
new release of the app.

## Layout

```
domestic/<country>.json   services expected to be reachable inside that country
foreign.json              services typically degraded or blocked at a country's border
```

The country is chosen by the user in Settings. There is no geo-detection: guessing the
user's country would mean asking someone else where they are, which this application never
does.

## Schema

```jsonc
{
  "schemaVersion": 1,
  "id": "ru",              // must match the file name
  "targets": [
    {
      "id": "yandex-dns",  // stable, unique within the list; used as a React key
      "label": "Yandex DNS", // a proper noun, shown as-is and not translated
      "address": "77.88.8.8", // an IP literal or a host name
      "port": 443          // optional; without it only ICMP can be used
    }
  ]
}
```

`address` may be a host name. It is resolved once, when monitoring starts, through the
system resolver — the same lookup any application on the machine makes. A name that does
not resolve is shown as unresolved rather than quietly dropped.

Host names are what make the foreign list meaningful. Public DNS resolvers are anycast:
`1.1.1.1` from Moscow usually terminates in Moscow, so it measures the domestic leg and
says almost nothing about the border. A name belonging to a service actually hosted abroad
resolves to a unicast address on the far side of that border, which is the thing the
foreign baseline exists to measure. Both kinds are present deliberately, and the anycast
entries double as a control: if the resolvers are fine and the named services are not, the
problem is not the user's ISP.

## Choosing entries

* Prefer addresses and names that are **published by their operator** — public resolvers,
  a service's own front door. Never anything observed on a developer's machine.
* Prefer a handful of diverse operators over many endpoints of one: four Yandex addresses
  measure Yandex, not Russia.
* Every entry costs probe budget. Baselines are probed at the interval in Settings
  (5 s by default) and share the global 32 probes/s cap with everything else, so keep each
  list around four entries.
