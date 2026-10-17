package refsim;

import org.eclipse.emf.ecore.util.EcoreUtil;

/**
 * Deterministic ids of PCM objects created by reconfiguration rules (patch P10, DESIGN-SPD R2).
 *
 * <p>Generated PCM code gives every new {@code Identifier} an {@code EcoreUtil.generateUUID()} in
 * its constructor ({@code IdentifierImpl}): time-based, so a rule that creates an object without
 * setting its id gives it a different id in every run. Ids reach the trace (assembly contexts,
 * actions, containers), the resource registry's {@code HashMap} order at the stop (container ids)
 * and string comparisons in rules.
 *
 * <p>The scheme: while a rule runs ({@link #open}/{@link #close} around each rule execution,
 * the shadowed {@code QVTOExecutor}) every new {@code Identifier} draws its id from one generator
 * per run: SplitMix64 seeded with {@link #SEED}, two draws per id (big-endian bytes 0-7, 8-15),
 * encoded as {@code EcoreUtil.generateUUID} encodes its 16 bytes ("_" + 22 characters of EMF's
 * base64 alphabet). Objects created outside rules (model loading, the runtime measurement model
 * and its measurements, SimuLizar's per-call system assembly contexts) keep EMF's UUIDs and do not
 * advance the generator. SimOxide's QVTo host draws the same sequence for the objects a rule
 * creates (the seeded generator of {@code simoxide_emf::IdPolicy}, {@code RunConfig::default()}).
 */
public final class Ids {
    /** SimOxide's default seed ({@code simoxide_reconf::RunConfig::default()}). */
    public static final long SEED = 0x5EED0F51L;

    private static final char[] DIGITS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
        .toCharArray();

    private static long state = SEED;
    private static int depth;
    private static long draws;

    private Ids() {
    }

    /** Before every run (batch runs equal fresh JVMs). */
    public static synchronized void reset() {
        state = SEED;
        depth = 0;
        draws = 0;
    }

    /** A rule starts running. */
    public static synchronized void open() {
        depth++;
    }

    /** The rule ended (also when it threw). */
    public static synchronized void close() {
        depth--;
    }

    /** Ids drawn in this run so far. */
    public static synchronized long draws() {
        return draws;
    }

    /** The id of a new {@code Identifier}: seeded while a rule runs, else EMF's UUID. */
    public static synchronized String generate() {
        if (depth <= 0) {
            return EcoreUtil.generateUUID();
        }
        draws++;
        byte[] b = new byte[16];
        long hi = next();
        long lo = next();
        for (int i = 0; i < 8; i++) {
            b[i] = (byte) (hi >>> (56 - 8 * i));
            b[8 + i] = (byte) (lo >>> (56 - 8 * i));
        }
        return encode(b);
    }

    private static long next() {
        state += 0x9E3779B97F4A7C15L;
        long z = state;
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    /** {@code EcoreUtil.UUID.generate}'s encoding of 16 bytes. */
    static String encode(byte[] u) {
        StringBuilder s = new StringBuilder(23);
        s.append('_');
        for (int i = 0; i < 5; i++) {
            int a = u[3 * i] & 0xFF, b = u[3 * i + 1] & 0xFF, c = u[3 * i + 2] & 0xFF;
            s.append(DIGITS[(a >> 2) & 0x3F]);
            s.append(DIGITS[((a << 4) & 0x30) | ((b >> 4) & 0x0F)]);
            s.append(DIGITS[((b << 2) & 0x3C) | ((c >> 6) & 0x03)]);
            s.append(DIGITS[c & 0x3F]);
        }
        int last = u[15] & 0xFF;
        s.append(DIGITS[(last >> 2) & 0x3F]);
        s.append(DIGITS[(last << 4) & 0x30]);
        return s.toString();
    }
}
