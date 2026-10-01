package schedoracle;

import java.io.PrintStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Consumer;

import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.core.composition.CompositionFactory;
import org.palladiosimulator.pcm.repository.PassiveResource;
import org.palladiosimulator.pcm.repository.RepositoryFactory;

import de.uka.ipd.sdq.scheduler.IActiveResource;
import de.uka.ipd.sdq.scheduler.IPassiveResource;
import de.uka.ipd.sdq.scheduler.ISchedulableProcess;
import de.uka.ipd.sdq.scheduler.SchedulerModel;
import de.uka.ipd.sdq.scheduler.entities.SchedulerEntity;
import de.uka.ipd.sdq.scheduler.resources.active.ResourceTableManager;
import de.uka.ipd.sdq.scheduler.resources.active.SimDelayResource;
import de.uka.ipd.sdq.scheduler.resources.active.SimFCFSResource;
import de.uka.ipd.sdq.scheduler.resources.active.SimProcessorSharingResource;
import de.uka.ipd.sdq.scheduler.sensors.IActiveResourceStateSensor;
import de.uka.ipd.sdq.scheduler.sensors.IPassiveResourceSensor;
import de.uka.ipd.sdq.simucomframework.core.resources.SimSimpleFairPassiveResource;
import de.uka.ipd.sdq.simulation.abstractsimengine.AbstractSimEventDelegator;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimEngineFactory;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimulationConfig;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimulationControl;
import de.uka.ipd.sdq.simulation.abstractsimengine.desmoj.DesmoJExperiment;
import de.uka.ipd.sdq.simulation.abstractsimengine.desmoj.DesmoJSimEngineFactory;

/**
 * Scheduler oracle: drives the real SimuLizar 5.2.2 resource classes (de.uka.ipd.sdq.scheduler
 * PS/FCFS/Delay and simucomframework SimSimpleFairPassiveResource) on the real DESMO-J engine
 * (nanosecond epsilon, as configured by DesmoJExperiment) with scripted jobs, and prints a
 * line-based trace. Mirrored by crates/simoxide-sched/tests/common/mod.rs.
 *
 * <pre>
 * Script:  resource ps|fcfs|delay CORES RATE      passive CAPACITY
 *          job NAME START DEMAND [THINK|now DEMAND]...        (demand / RATE is issued)
 *          pjob NAME START NUM HOLD [THINK|now NUM HOLD]...
 * Trace (t = DESMO-J time in ns, x = hex bits of a double):
 *   D t job x      demand issued        Z t job      demand <= 0, skipped
 *   S t core n     state change         C t job      demand completed
 *   A t job        process activated    E t job      job finished its script
 *   R t job=x ...  remaining demands after each scheduler call (sorted by name)
 *   Q/G/L t job n  passive request/acquire/release, V t available [granted|queued]
 * </pre>
 * A THINK is a separate event after THINK seconds; "now" continues in the resume event.
 */
public class Main {

    static PrintStream out;
    static OModel model;

    // ---------------------------------------------------------------- engine plumbing

    static final class OModel extends SchedulerModel {
        private final ISimulationConfig cfg = () -> "schedoracle";
        private ISimEngineFactory fac;
        private ISimulationControl ctl;
        private int initCalls;
        Runnable initial = () -> {};

        OModel() {
            fac = new DesmoJSimEngineFactory();
            fac.setModel(this);
            ctl = fac.createSimulationControl();
        }

        @Override public ISimulationControl getSimulationControl() { return ctl; }
        @Override public void setSimulationControl(ISimulationControl c) { ctl = c; }
        @Override public void setSimulationEngineFactory(ISimEngineFactory f) { fac = f; }
        @Override public ISimEngineFactory getSimEngineFactory() { return fac; }
        @Override public ISimulationConfig getConfiguration() { return cfg; }

        /** Called by AbstractExperiment.start() before DESMO-J's experiment starts (clock = 0). */
        @Override public void init() {
            if (++initCalls == 1) {
                initial.run();
            }
        }

        @Override public void finalise() {}
    }

    static long now() {
        return ((DesmoJExperiment) model.getSimulationControl()).getExperiment().getSimClock().getTime()
            .getTimeInEpsilon();
    }

    static final class Ev extends AbstractSimEventDelegator<Proc> {
        private final Consumer<Proc> r;

        Ev(String name, Consumer<Proc> r) {
            super(model, name);
            this.r = r;
        }

        @Override public void eventRoutine(Proc p) { r.accept(p); }
    }

    /** Schedules a fresh one-shot event (like DesmoJSimProcess.scheduleAt / passivate(delay)). */
    static void at(Proc p, double delay, Runnable r) {
        new Ev("ev", x -> r.run()).schedule(p, delay);
    }

    static final class Proc extends SchedulerEntity implements ISchedulableProcess {
        final String name;
        Runnable next;

        Proc(String name) {
            super(model, name);
            this.name = name;
        }

        /** Like SimuComSimProcess.activate(): scheduleAt(0) -> resume event at now + 0. */
        @Override public void activate() {
            out.println("A " + now() + " " + name);
            dumpRemaining();
            final Runnable n = next;
            next = null;
            at(this, 0.0, n);
        }

        @Override public void passivate() {}
        @Override public String getId() { return name; }
        @Override public ISchedulableProcess getRootProcess() { return this; }
        @Override public boolean isFinished() { return false; }
        @Override public void fireTerminated() {}
        @Override public void addTerminatedObserver(IActiveResource o) {}
        @Override public void removeTerminatedObserver(IActiveResource o) {}
        @Override public int getPriority() { return 0; }
        @Override public void setPriority(int prio) {}
        @Override public void timeout(String timeoutFailureName) {}
        @Override public String toString() { return name; }
    }

    // ---------------------------------------------------------------- script

    /** One step: optional think time before it (null = continue immediately), then the step. */
    record Step(Double think, double a, double b) {}

    record Job(String name, double start, List<Step> steps) {}

    static String kind;
    static long cores = 1;
    static double rate = 1.0;
    static long capacity = 1;
    static final List<Job> jobs = new ArrayList<>();

    static void parse(Path script) throws Exception {
        for (String raw : Files.readAllLines(script)) {
            String line = raw.strip();
            int hash = line.indexOf('#');
            if (hash >= 0) line = line.substring(0, hash).strip();
            if (line.isEmpty()) continue;
            String[] t = line.split("\\s+");
            switch (t[0]) {
            case "resource" -> {
                kind = t[1];
                cores = Long.parseLong(t[2]);
                rate = Double.parseDouble(t[3]);
            }
            case "passive" -> {
                kind = "passive";
                capacity = Long.parseLong(t[1]);
            }
            case "job" -> {
                // job <name> <start> <demand> [<think|now> <demand>]...
                List<Step> steps = new ArrayList<>();
                steps.add(new Step(null, Double.parseDouble(t[3]), 0));
                for (int i = 4; i + 1 < t.length; i += 2) {
                    steps.add(new Step(think(t[i]), Double.parseDouble(t[i + 1]), 0));
                }
                jobs.add(new Job(t[1], Double.parseDouble(t[2]), steps));
            }
            case "pjob" -> {
                // pjob <name> <start> <num> <hold> [<think|now> <num> <hold>]...
                List<Step> steps = new ArrayList<>();
                steps.add(new Step(null, Double.parseDouble(t[3]), Double.parseDouble(t[4])));
                for (int i = 5; i + 2 < t.length; i += 3) {
                    steps.add(new Step(think(t[i]), Double.parseDouble(t[i + 1]), Double.parseDouble(t[i + 2])));
                }
                jobs.add(new Job(t[1], Double.parseDouble(t[2]), steps));
            }
            default -> throw new IllegalArgumentException("bad line: " + raw);
            }
        }
    }

    static Double think(String s) {
        return s.equals("now") ? null : Double.valueOf(s);
    }

    static String hex(double d) {
        return Long.toHexString(Double.doubleToRawLongBits(d));
    }

    // ---------------------------------------------------------------- active resources

    static IActiveResource res;
    static Field remField;

    static void dumpRemaining() {
        if (remField == null) return;
        try {
            @SuppressWarnings("unchecked")
            Map<ISchedulableProcess, Double> m = (Map<ISchedulableProcess, Double>) remField.get(res);
            TreeMap<String, Double> sorted = new TreeMap<>();
            for (Map.Entry<ISchedulableProcess, Double> e : m.entrySet()) {
                sorted.put(((Proc) e.getKey()).name, e.getValue());
            }
            StringBuilder sb = new StringBuilder("R ").append(now());
            sorted.forEach((k, v) -> sb.append(' ').append(k).append('=').append(hex(v)));
            out.println(sb);
        } catch (IllegalAccessException e) {
            throw new RuntimeException(e);
        }
    }

    static void issue(Proc p, Job j, int i) {
        double concrete = j.steps.get(i).a / rate; // ScheduledResource.calculateDemand
        out.println("D " + now() + " " + p.name + " " + hex(concrete));
        if (concrete <= 0) { // AbstractScheduledResource.consumeResource: skip, thread continues
            out.println("Z " + now() + " " + p.name);
            afterStep(p, j, i);
            return;
        }
        p.next = () -> afterStep(p, j, i);
        res.process(p, 0, Collections.emptyMap(), concrete);
        dumpRemaining();
    }

    static void afterStep(Proc p, Job j, int i) {
        if (i + 1 >= j.steps.size()) {
            out.println("E " + now() + " " + p.name);
            return;
        }
        Step s = j.steps.get(i + 1);
        if (s.think == null) {
            issue(p, j, i + 1);
        } else {
            at(p, s.think, () -> issue(p, j, i + 1));
        }
    }

    // ---------------------------------------------------------------- passive resource

    static IPassiveResource pres;

    static void acquireStep(Proc p, Job j, int i) {
        Step s = j.steps.get(i);
        boolean ok = pres.acquire(p, (long) s.a, false, 0.0);
        out.println("V " + now() + " " + pres.getAvailable() + " " + (ok ? "granted" : "queued"));
        if (ok) {
            hold(p, j, i);
        } else {
            p.next = () -> hold(p, j, i);
        }
    }

    static void hold(Proc p, Job j, int i) {
        Step s = j.steps.get(i);
        at(p, s.b, () -> {
            pres.release(p, (long) s.a);
            out.println("V " + now() + " " + pres.getAvailable());
            if (i + 1 >= j.steps.size()) {
                out.println("E " + now() + " " + p.name);
                return;
            }
            Step n = j.steps.get(i + 1);
            if (n.think == null) {
                acquireStep(p, j, i + 1);
            } else {
                at(p, n.think, () -> acquireStep(p, j, i + 1));
            }
        });
    }

    // ---------------------------------------------------------------- main

    public static void main(String[] args) throws Exception {
        out = new PrintStream(System.out, false, "UTF-8");
        parse(Path.of(args[0]));
        model = new OModel();
        switch (kind) {
        case "ps" -> res = new SimProcessorSharingResource(model, "PS", "1", cores, new ResourceTableManager());
        case "fcfs" -> res = new SimFCFSResource(model, "FCFS", "1", cores, new ResourceTableManager());
        case "delay" -> res = new SimDelayResource(model, "DELAY", "1", new ResourceTableManager());
        case "passive" -> {
            PassiveResource pr = RepositoryFactory.eINSTANCE.createPassiveResource();
            pr.setEntityName("P");
            pr.setId("p1");
            AssemblyContext ac = CompositionFactory.eINSTANCE.createAssemblyContext();
            ac.setId("ac1");
            pres = new SimSimpleFairPassiveResource(pr, ac, model, capacity);
            pres.addObserver(new IPassiveResourceSensor() {
                @Override public void request(ISchedulableProcess p, long num) {
                    out.println("Q " + now() + " " + p.getId() + " " + num);
                }
                @Override public void acquire(ISchedulableProcess p, long num) {
                    out.println("G " + now() + " " + p.getId() + " " + num);
                }
                @Override public void release(ISchedulableProcess p, long num) {
                    out.println("L " + now() + " " + p.getId() + " " + num);
                }
            });
        }
        default -> throw new IllegalArgumentException("unknown resource kind " + kind);
        }
        if (res != null) {
            if (!(res instanceof SimDelayResource)) {
                remField = res.getClass().getDeclaredField("running_processes");
                remField.setAccessible(true);
            }
            res.addObserver(new IActiveResourceStateSensor() {
                @Override public void update(long state, int instanceId) {
                    out.println("S " + now() + " " + instanceId + " " + state);
                }
                @Override public void demandCompleted(ISchedulableProcess p) {
                    out.println("C " + now() + " " + p.getId());
                }
            });
            res.start();
        }
        out.println("# " + kind + " cores=" + cores + " rate=" + hex(rate) + " capacity=" + capacity);
        if (res != null) {
            System.err.println("impl: " + res.getClass().getProtectionDomain().getCodeSource().getLocation());
        }
        model.initial = () -> {
            for (Job j : jobs) {
                Proc p = new Proc(j.name);
                if (pres != null) {
                    at(p, j.start, () -> acquireStep(p, j, 0));
                } else {
                    at(p, j.start, () -> issue(p, j, 0));
                }
            }
        };
        model.getSimulationControl().start();
        out.println("# end " + now());
        out.flush();
        System.exit(0);
    }
}
