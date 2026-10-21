<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: MIT
-->

# Sensor observation interface 0.1 (draft)

How a small device (a sensor hub, a single sensor bridge) publishes readings
and coverage to a service that keeps them. Provider-neutral and MIT-licensed,
so a kit can implement it under any license without touching GPL code.

- `schema.json`: JSON Schema 2020-12 definitions (`$defs`) for every shape.
- `example.json`: a synthetic batch, its result and the channels it refers to.

**Status:** draft, October 21, 2026. Daybook's proposed satellite contract
implements the same definitions; neither side has shipped them yet. A change
to one changes both in the same release and bumps the version here.

## Concepts

| Term | Meaning |
| --- | --- |
| Source | One physical sensor, registered by its owner in the receiving service. Identified by a UUID the service assigns |
| Channel | One measured quantity of a source (`temperature`, `battery`, `contact` and so on), with a fixed key, quantity, unit and value kind |
| Observation | One reading: source, channel, device-assigned `native_id`, `event_at`, clock state, value, quality |
| Coverage window | The device's statement that it was collecting the listed channels for a whole period |
| Publisher | The device that sends observations, holding a credential bound to the sources it may write |

## Shapes

`SensorChannel`, `SensorObservation`, `CoverageWindow`,
`SensorObservationBatch`, `SensorItemResult`, `SensorBatchResult`, `Period`,
plus the closed `SensorQuantity` and `SensorUnit` lists. Units are UCUM codes;
null for boolean and state channels.

| Quantity | Value kind | Units |
| --- | --- | --- |
| `temperature` | number | `Cel`, `[degF]` |
| `relative_humidity`, `soil_moisture`, `battery` | number | `%` |
| `pressure` | number | `hPa` |
| `illuminance` | number | `lx` |
| `co2` | number | `ppm` |
| `pm2_5` | number | `ug/m3` |
| `power`, `energy`, `voltage` | number | `W`, `kW.h`, `V` |
| `odometer` | number | `km`, `[mi_i]` |
| `contact` | boolean | none (`true` closed) |
| `motion`, `occupancy`, `water_leak`, `vibration` | boolean | none (`true` detected) |

Smoke, carbon monoxide and intrusion are deliberately absent. This interface
is for household information, not life-safety alarms.

## Rules a conforming publisher follows

1. **Idempotency.** `native_id` is unique per source for ever (for example a
   boot id plus a sequence number). Resending a reading is safe: the receiver
   stores it once and reports `duplicate`.
2. **Order.** Send oldest first. Receivers order by `event_at`, so a late
   reading never replaces a newer one as the current value.
3. **Buffering.** When the receiver is unreachable, keep readings and send them
   later. If the buffer overflows, drop the oldest and leave that time
   uncovered rather than claiming it.
4. **Clocks.** Send `clock: synced` only when the device trusts its time.
   Readings marked `unsynced` are stamped with the receipt time, so send them
   live or not at all. A synced reading more than 5 minutes in the future is
   rejected with `clock_skew`.
5. **Coverage.** Channels that report only on change send a coverage window at
   least once per `expected_interval_seconds`. Time inside a window with no
   reading means nothing was observed; time outside every window is unknown.
   A window is at most 24 hours and ends no later than 5 minutes after receipt.
6. **Values.** A value matches its channel's value kind. A `state` channel
   sends only its declared state names. There is no free text in an
   observation.
7. **Limits.** At most 500 observations and 100 coverage windows per batch,
   256 KiB per body.

## Rules a conforming receiver follows

1. Accept or report every item. A success response means every accepted item
   is durably stored; every other item appears in `items` with its status.
2. Refuse the whole batch, storing nothing, if any `source_id` is not bound to
   the caller's credential. Do not distinguish unbound from nonexistent.
3. `discarded_paused` and `duplicate` are final. `rejected` with
   `source_unavailable` tells the publisher to stop sending for that source.
4. Treat observations as untrusted data. They may update observed state and
   feed rules the owner set; they never complete work, approve anything, or
   reach a model as instructions.
5. Report freshness honestly: a channel silent beyond 3 times its expected
   interval is unknown, never "none" or "OK".

## HTTP binding

```text
POST {base}/sensors/observations
Authorization: Bearer <credential scoped to write the batch's sources>
Idempotency-Key: <16 to 128 characters, stable per logical batch>
Content-Type: application/json

body:     SensorObservationBatch
200:      SensorBatchResult
403:      a source is not bound to this credential (nothing stored)
413, 429: limits; 429 carries Retry-After
```

Requests must not carry an `Origin` header; receivers refuse them, so a web
page cannot use a device credential. How a device obtains its credential is
the receiver's business; Daybook's enrollment is one example.

## Conformance checks

- Every example in `example.json` validates against `schema.json`.
- A publisher test resends a batch and expects only `duplicate` results.
- A publisher test with the receiver unreachable for longer than the expected
  interval shows the gap as uncovered afterwards.
- A receiver test sends a batch with one unbound source and expects 403 and no
  stored items.

## Implementations

| Implementation | Role | State |
| --- | --- | --- |
| Daybook (private) | Receiver | Proposed, not built |
