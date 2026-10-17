# simoxide-jcompat-exact

Java behaviour that only OpenJDK's and HotSpot's sources define, for SimOxide's exact mode
(SimuLizar-exact) and the QVTo engine's compat mode: `java.util.HashMap`'s iteration order
(`hashmap`, `order`) and HotSpot's identity-hash generator (`hotspot`).

**No licence is granted for this crate, and builds that include it must not be distributed**
(`LICENSE-NOTICE`). The default builds of SimOxide and of the QVTo engine do not include it;
the cargo features `jdk-order`, `exact` and `compat` opt in.

The EPL-2.0 counterpart, with what the Java documentation specifies, is `simoxide-jcompat`.
