---
layout: home

hero:
  name: SimOxide
  text: SimuLizar 5.2.2, byte for byte, in Rust
  tagline: The Palladio performance simulator, reproduced exactly and two to three orders of magnitude faster.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Performance
      link: /performance/
    - theme: alt
      text: Exactness
      link: /correctness/
    - theme: alt
      text: GitHub
      link: https://github.com/nk-coding/simoxide

features:
  - title: Byte-identical
    details: The same event trace, random draws and measurements as the reference for every supported model, reference bugs included. Checked on 66 corpus models and about 12 000 generated models.
  - title: 240 to 3 300 times faster
    details: Than stock SimuLizar 5.2.2 per warm run, on either of its simulation engines (DESMO-J or SSJ). A one-shot run takes milliseconds instead of seconds; 22 cores complete over 5 000 MediaStore evaluations per second.
  - title: Small and stable
    details: A few MB per process, no warm-up, no leaks, no global state. 12.6x on 22 threads.
  - title: Embeddable
    details: A library that loads models from memory, runs batches in parallel and bounds untrusted input with limits.
  - title: Fast mode
    details: An opt-in mode with statistically equivalent, faster random numbers, up to 3.4 times faster on distribution-heavy models.
  - title: Specified
    details: The semantics of SimuLizar 5.2.2, extracted from its sources with file and line references, as numbered rules.
  - title: A QVTo engine
    details: SimuLizar's reconfiguration rules and Slingshot's SPD adjustments run on a Rust QVTo engine that equals Eclipse QVTo on all 589 runnable conformance cases (in its compat mode) and runs a rule in about 0.1 ms.
    link: /qvto/
    linkText: QVTo engine
---
