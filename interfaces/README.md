# Contracts and interfaces

Versioned contract and interface definitions that Diamond Dog components and
services implement. Everything in this folder is licensed under the MIT
License (see [LICENSE](LICENSE)), so anyone can implement them under any
license.

## Contents

- [diamond-dog/provisional-contract.md](diamond-dog/provisional-contract.md):
  the provisional contract that products built on top of the runtime (first,
  Orca Keel) pin against: ten surfaces, each versioned on its own, plus the
  fake runtime that stands in until the real one exists. Provisional until the
  experiment gate; see its Status section.
- [diamond-dog/http-binding.md](diamond-dog/http-binding.md): how calls to
  those surfaces travel over HTTP, as the fake Diamond Dog answers them. The
  contract wins where the two differ.
- [diamond-dog/fake-fixtures.md](diamond-dog/fake-fixtures.md): the JSON
  format for test scenarios run against the fake, and its test controls.
- [sensor-observation](sensor-observation/README.md): version 0.1, draft. How
  a small device (a sensor hub, a single sensor bridge) publishes readings and
  coverage to a service that keeps them.
