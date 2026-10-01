package refsim;

import java.lang.reflect.Method;

import refsim.trace.Trace;

/**
 * Resets JVM-global state between runs so that run N in a batch JVM behaves exactly like the first
 * run in a fresh JVM. Patched (shadowed) classes expose a static {@code refsimReset()} method; in
 * the unpatched build those methods do not exist and nothing is reset.
 */
public final class Statics {
    static final String[] RESETTABLE = {
            "de.uka.ipd.sdq.simulation.abstractsimengine.AbstractSimProcessDelegator",
            "de.uka.ipd.sdq.simucomframework.core.SimuComSimProcess",
            "de.uka.ipd.sdq.simucomframework.core.SimuComDefaultRandomNumberGenerator",
    };

    private Statics() {
    }

    /** Private static counters reset by reflection (no class shadowing needed): class, field, value. */
    static final Object[][] COUNTERS = {
            { "de.uka.ipd.sdq.simucomframework.core.resources.ScheduledResource", "resourceId", 1L },
            { "de.uka.ipd.sdq.simucomframework.core.resources.SimulatedLinkingResource", "resourceId", 1L },
    };

    /** -Drefsim.unpatched=true (REFSIM_UNPATCHED=1): stock behaviour, no state resets either. */
    public static final boolean UNPATCHED = Boolean.getBoolean("refsim.unpatched");

    static void resetForNewRun() {
        if (UNPATCHED) {
            return;
        }
        for (Object[] c : COUNTERS) {
            try {
                java.lang.reflect.Field f = Class.forName((String) c[0]).getDeclaredField((String) c[1]);
                f.setAccessible(true);
                f.set(null, c[2]);
            } catch (ReflectiveOperationException e) {
                throw new RuntimeException(e);
            }
        }
        for (String cn : RESETTABLE) {
            try {
                Class<?> c = Class.forName(cn);
                Method m = c.getDeclaredMethod("refsimReset");
                m.invoke(null);
            } catch (NoSuchMethodException e) {
                // unpatched build
            } catch (ReflectiveOperationException e) {
                throw new RuntimeException(e);
            }
        }
    }

    static void afterRun() {
    }

    static double currentTime() {
        return Trace.now();
    }

    static boolean patched() {
        try {
            Class.forName("de.uka.ipd.sdq.simulation.abstractsimengine.AbstractSimProcessDelegator")
                .getDeclaredMethod("refsimReset");
            return true;
        } catch (ReflectiveOperationException e) {
            return false;
        }
    }
}
