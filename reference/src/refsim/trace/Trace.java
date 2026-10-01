package refsim.trace;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.Writer;
import java.util.IdentityHashMap;

import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EStructuralFeature;

/**
 * Event trace + random tape emitted by the patched reference (format: docs/guide/formats.md).
 *
 * <p>
 * Static on purpose: the patched simulator classes call it directly. SimuLizar runs exactly one
 * simulated process at a time (threads hand over through semaphores, which gives happens-before),
 * so no locking is needed. {@link #ON}/{@link #TAPE} are checked at every call site so a disabled
 * trace costs one static boolean read.
 */
public final class Trace {

    /** true while a trace sink is attached. */
    public static boolean ON;
    /** true while a tape sink is attached. */
    public static boolean TAPE;

    private static Writer trace;
    private static Writer tape;
    private static final StringBuilder SB = new StringBuilder(256);

    /** sim clock accessor of the current run. */
    public interface Clock {
        double now();
    }

    private static Clock clock = () -> 0.0;

    // ---- process ids: assigned in creation order, per run, starting at 1 ----
    private static final IdentityHashMap<Object, Integer> PIDS = new IdentityHashMap<>();
    private static int nextPid;
    /** process currently executing (set on resume), 0 = simulator/event context. */
    private static long seq;

    // ---- random tape state ----
    private static long uniformIndex;
    private static String origin = "?";
    private static int evalDepth;
    /** StoEx strings longer than this are truncated in tape sample records. */
    public static final int SPEC_MAX = 64;
    private static long evalStartIndex;

    private Trace() {
    }

    public static void begin(Writer traceOut, Writer tapeOut, Clock c) {
        trace = traceOut == null ? null : new BufferedWriter(traceOut, 1 << 16);
        tape = tapeOut == null ? null : new BufferedWriter(tapeOut, 1 << 16);
        ON = trace != null;
        TAPE = tape != null;
        clock = c;
        clockBound = false;
        PIDS.clear();
        nextPid = 0;
        seq = 0;
        uniformIndex = 0;
        origin = "?";
        evalDepth = 0;
    }

    public static void setClock(Clock c) {
        clock = c;
    }

    public static double now() {
        return clock.now();
    }

    public static void end() throws IOException {
        if (trace != null) {
            trace.flush();
        }
        if (tape != null) {
            tape.flush();
        }
        trace = null;
        tape = null;
        ON = false;
        TAPE = false;
        PIDS.clear();
        clock = () -> 0.0;
    }

    public static long uniformCount() {
        return uniformIndex;
    }

    // ------------------------------------------------------------------ formatting

    /** Canonical number format: Java (&gt;=19) Double.toString = shortest round-trip repr. */
    public static String num(double d) {
        if (Double.isNaN(d) || Double.isInfinite(d)) {
            return "\"" + d + "\"";
        }
        return Double.toString(d);
    }

    private static void str(StringBuilder sb, String s) {
        sb.append('"');
        if (s == null) {
            sb.append("null\"");
            return;
        }
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
            case '"':
                sb.append("\\\"");
                break;
            case '\\':
                sb.append("\\\\");
                break;
            case '\n':
                sb.append("\\n");
                break;
            case '\r':
                sb.append("\\r");
                break;
            case '\t':
                sb.append("\\t");
                break;
            default:
                if (c < 0x20) {
                    sb.append(String.format("\\u%04x", (int) c));
                } else {
                    sb.append(c);
                }
            }
        }
        sb.append('"');
    }

    private static StringBuilder start(String ev) {
        StringBuilder sb = SB;
        sb.setLength(0);
        sb.append("{\"ev\":\"").append(ev).append("\",\"t\":").append(num(clock.now()));
        return sb;
    }

    private static StringBuilder kv(StringBuilder sb, String k, String v) {
        sb.append(",\"").append(k).append("\":");
        str(sb, v);
        return sb;
    }

    private static StringBuilder kv(StringBuilder sb, String k, long v) {
        return sb.append(",\"").append(k).append("\":").append(v);
    }

    private static StringBuilder kvd(StringBuilder sb, String k, double v) {
        return sb.append(",\"").append(k).append("\":").append(num(v));
    }

    private static void emit(StringBuilder sb) {
        sb.append("}\n");
        try {
            trace.write(sb.toString());
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
    }

    public static String id(EObject o) {
        if (o == null) {
            return null;
        }
        EStructuralFeature f = o.eClass().getEStructuralFeature("id");
        if (f != null) {
            Object v = o.eGet(f);
            if (v != null) {
                return v.toString();
            }
        }
        return org.eclipse.emf.ecore.util.EcoreUtil.getURI(o).fragment();
    }

    // ------------------------------------------------------------------ processes

    private static final ThreadLocal<Object> CURRENT = new ThreadLocal<>();

    /** Binds the trace clock to the simulation control of the current run (first call per run wins). */
    public static void bindControl(de.uka.ipd.sdq.simulation.abstractsimengine.ISimulationControl c) {
        if (!clockBound && c != null) {
            clock = c::getCurrentSimulationTime;
            clockBound = true;
        }
    }

    private static boolean clockBound;

    /** Marks the calling thread as executing the given simulated process. */
    public static void enterProcess(Object p) {
        CURRENT.set(p);
    }

    public static void exitProcess() {
        CURRENT.remove();
    }

    /** The simulated process executing on the calling thread (null: simulator/event thread). */
    public static Object currentProcess() {
        return CURRENT.get();
    }

    /** Registers a new simulated process (called from its constructor). Returns its trace id. */
    public static int newProcess(Object process, String name) {
        int pid = ++nextPid;
        PIDS.put(process, pid);
        if (ON) {
            StringBuilder sb = start("spawn");
            kv(sb, "p", pid);
            kv(sb, "kind", kindOf(process));
            kv(sb, "name", name);
            kv(sb, "parent", pid(CURRENT.get()));
            emit(sb);
        }
        return pid;
    }

    private static String kindOf(Object o) {
        Class<?> c = o.getClass();
        while (c.isAnonymousClass()) {
            c = c.getSuperclass();
        }
        return c.getSimpleName();
    }

    public static int pid(Object process) {
        if (process == null) {
            return 0;
        }
        Integer p = PIDS.get(process);
        return p == null ? -1 : p;
    }

    public static void processEnd(Object process) {
        if (ON) {
            StringBuilder sb = start("pend");
            kv(sb, "p", pid(process));
            emit(sb);
        }
    }

    // ------------------------------------------------------------------ generic event

    /** Generic event with a process and a model element (+ optional assembly context). */
    public static void element(String ev, Object process, String type, String id, String ac) {
        StringBuilder sb = start(ev);
        kv(sb, "p", pid(process));
        kv(sb, "type", type);
        kv(sb, "id", id);
        if (ac != null) {
            kv(sb, "ac", ac);
        }
        emit(sb);
    }

    public static void event(String ev, Object process, Object... kvs) {
        StringBuilder sb = start(ev);
        if (process != null) {
            kv(sb, "p", pid(process));
        }
        for (int i = 0; i + 1 < kvs.length; i += 2) {
            String k = (String) kvs[i];
            Object v = kvs[i + 1];
            if (v == null) {
                continue;
            }
            if (v instanceof Double || v instanceof Float) {
                kvd(sb, k, ((Number) v).doubleValue());
            } else if (v instanceof Number) {
                kv(sb, k, ((Number) v).longValue());
            } else if (v instanceof Boolean) {
                sb.append(",\"").append(k).append("\":").append(v);
            } else {
                kv(sb, k, v.toString());
            }
        }
        emit(sb);
    }

    public static void measurement(String mp, String metric, double[] row) {
        StringBuilder sb = start("meas");
        kv(sb, "mp", mp);
        kv(sb, "metric", metric);
        sb.append(",\"v\":[");
        for (int i = 0; i < row.length; i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(num(row[i]));
        }
        sb.append(']');
        emit(sb);
    }

    public static void raw(String jsonLine) {
        try {
            trace.write(jsonLine);
            trace.write('\n');
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
    }

    // ------------------------------------------------------------------ random tape

    /** Sets the origin tag for the following draws; returns the previous one (restore it after). */
    public static String origin(String o) {
        String prev = origin;
        origin = o;
        return prev;
    }

    /** Sets origin "purpose:elementId"; returns the previous origin (restore it in a finally block). */
    public static String origin(String purpose, EObject element) {
        if (!TAPE) { // origins are only written to the tape: skip the id lookup
            return origin;
        }
        return origin(element == null ? purpose : purpose + ":" + id(element));
    }

    public static String currentOrigin() {
        return origin;
    }

    /** Called for every uniform drawn from the (single) simulation RNG stream. */
    public static void uniform(double u) {
        long i = uniformIndex++;
        if (TAPE) {
            StringBuilder sb = new StringBuilder(96);
            sb.append("{\"k\":\"u\",\"i\":").append(i).append(",\"u\":").append(num(u)).append(",\"o\":");
            str(sb, origin);
            sb.append("}\n");
            try {
                tape.write(sb.toString());
            } catch (IOException e) {
                throw new RuntimeException(e);
            }
        }
    }

    /** Start of a StoEx evaluation (nesting allowed). */
    public static long evalBegin() {
        if (evalDepth++ == 0) {
            evalStartIndex = uniformIndex;
        }
        return uniformIndex;
    }

    /**
     * End of a StoEx evaluation. If the outermost evaluation consumed uniforms, a sample record with
     * the derived value is written to the tape.
     */
    public static void evalEnd(String spec, Object value) {
        if (--evalDepth == 0 && TAPE && uniformIndex > evalStartIndex) {
            StringBuilder sb = new StringBuilder(128);
            sb.append("{\"k\":\"s\",\"n\":").append(uniformIndex - evalStartIndex).append(",\"o\":");
            str(sb, origin);
            sb.append(",\"spec\":");
            str(sb, spec == null || spec.length() <= SPEC_MAX ? spec : spec.substring(0, SPEC_MAX - 3) + "...");
            sb.append(",\"v\":");
            if (value instanceof Double || value instanceof Float) {
                sb.append(num(((Number) value).doubleValue()));
            } else if (value instanceof Number || value instanceof Boolean) {
                sb.append(value);
            } else {
                str(sb, String.valueOf(value));
            }
            sb.append("}\n");
            try {
                tape.write(sb.toString());
            } catch (IOException e) {
                throw new RuntimeException(e);
            }
        }
    }

    /** Abort handling: resets eval nesting (exception inside evaluation). */
    public static void evalAbort() {
        if (evalDepth > 0) {
            evalDepth--;
        }
    }
}
