---
layout: home

hero:
  name: SimOxide
  text: SimuLizar and Slingshot, byte for byte, in Rust
  tagline: The Palladio performance simulator, reproduced exactly and two to three orders of magnitude faster.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Modes and features
      link: /guide/modes
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
  - title: 250 to 2 400 times faster
    details: Than stock SimuLizar 5.2.2 per warm run. A one-shot run takes milliseconds instead of seconds; 22 cores complete over 5 000 MediaStore evaluations per second.
  - title: Small and stable
    details: A few MB per process, no warm-up, no leaks, no global state. 13x on 22 threads.
  - title: Slingshot-compatible
    details: Slingshot's semantics with its bugs fixed or reproduced flag by flag, or its event bus replayed handler by handler, byte-equal to the deterministic Slingshot on its corpus.
    link: /guide/slingshot
    linkText: Slingshot mode
  - title: Autoscaling
    details: Slingshot's scaling policies (SPD) with its own QVTo adjustments, Kubernetes-style HPA autoscalers, SimuLizar's reconfiguration rules, and time-varying load profiles.
    link: /guide/spd
    linkText: Scaling policies
  - title: Snapshots and forks
    details: Pause a run between two events, fork it, write it to a file, change the model, and continue elsewhere, byte-equal to the uninterrupted run.
    link: /guide/snapshots
    linkText: Snapshots
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
