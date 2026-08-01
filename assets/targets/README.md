# Target lists

These files are the *only* places the app learns what to probe. They are plain JSON,
versioned in this repository, and readable by anyone who wants to audit exactly which
addresses the application will contact. Nothing here is fetched at runtime and nothing is
updated behind the user's back: a new list ships with a new release of the app.

## Layout

```
domestic/<country>.json   services expected to be reachable inside that country
foreign.json              services typically degraded or blocked at a country's border
services.json             the status page: platforms and infrastructure, grouped
```

The country is chosen by the user in Settings. There is no geo-detection: guessing the
user's country would mean asking someone else where they are, which this application never
does.

## The baselines: `domestic/<country>.json` and `foreign.json`

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

### Choosing baseline entries

* Prefer addresses and names that are **published by their operator** — public resolvers,
  a service's own front door. Never anything observed on a developer's machine.
* Prefer a handful of diverse operators over many endpoints of one: four Yandex addresses
  measure Yandex, not Russia.
* Every entry costs probe budget. Baselines are probed at the interval in Settings
  (5 s by default) and share the global 32 probes/s cap with everything else, so keep each
  list around four entries.

## The status page: `services.json`

The dashboard's baselines ask *how is my connection*. The status page asks *is it them or
me* — whether this machine can reach the platforms and the infrastructure a player depends
on. That difference is why it is a separate file with a schema of its own, and why it is
judged by a rule of its own (`nm_core::status`): a card says whether a service answers
**now**, so it reacts within one check, while a baseline reports what a window has been
like.

```jsonc
{
  "schemaVersion": 1,
  "services": [
    {
      "id": "steam",              // stable, unique in the file; a React key
      "label": "Steam",           // a proper noun, shown as-is and not translated
      "group": "gamingPlatform",  // or "infrastructure"
      "probeKind": "tcpConnect",  // optional; see below
      "endpoints": [
        { "id": "api", "address": "api.steampowered.com", "port": 443 }
      ]
    }
  ]
}
```

An endpoint carries **no label**, unlike a baseline entry. It already sits under the
operator's name on the card, and the written address — `store.steampowered.com` beside
`api.steampowered.com` — says what it is better than any word we could put there, in every
language, without a translation.

### Rules for an entry

* **Names, not addresses.** A platform's front door lives on a content network whose
  address depends on where the user is; pinning one in a bundled file would measure
  whichever edge the *developer* was nearest and would go stale silently. A test enforces
  this.
* **A published front door, and only one or two of them.** Enough to tell "the storefront
  is up but the gateway is not" apart, never enough to survey a company's estate.
* **Every endpoint carries a port.** Without one only ICMP can be used, and a front door
  that drops echoes would then have no fallback at all — which on a status page means a
  permanently red card about a service that is up.
* **The whole list must stay negligible.** These checks run whether or not the user is
  doing anything, at one check per endpoint every 45 seconds. A test asserts the list costs
  under one probe a second against the product's cap of thirty-two.

### `probeKind` is a hint, never a permission

It names the kind to **try first**. It reorders the kinds the address class already allows
and can never introduce one it refuses — a tunnelled endpoint still gets the end-to-end
probe whatever this field says, so a hand-edited list can shorten a wait and can never make
the app report a figure a tunnel invented. A kind this build has no prober for is ignored
rather than fatal.

Why it is worth having: without it the fallback chain opens on the cheapest kind and needs
three silent checks — over two minutes at this cadence — before trying the one that works.
The bundled entries name `tcpConnect`, because a front door is *defined* by its port
answering, which is the question the card asks; whether the operator's edge router echoes a
ping is a different question.
