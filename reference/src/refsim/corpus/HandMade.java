package refsim.corpus;

import static refsim.corpus.PcmBuilder.BYTESIZE;
import static refsim.corpus.PcmBuilder.NUMBER_OF_ELEMENTS;
import static refsim.corpus.PcmBuilder.VALUE;
import static refsim.corpus.PcmBuilder.vu;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;
import java.util.function.Function;

import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.repository.BasicComponent;
import org.palladiosimulator.pcm.repository.CompositeComponent;
import org.palladiosimulator.pcm.repository.InfrastructureInterface;
import org.palladiosimulator.pcm.repository.InfrastructureProvidedRole;
import org.palladiosimulator.pcm.repository.InfrastructureRequiredRole;
import org.palladiosimulator.pcm.repository.InfrastructureSignature;
import org.palladiosimulator.pcm.repository.OperationInterface;
import org.palladiosimulator.pcm.repository.OperationProvidedRole;
import org.palladiosimulator.pcm.repository.OperationRequiredRole;
import org.palladiosimulator.pcm.repository.OperationSignature;
import org.palladiosimulator.pcm.repository.Parameter;
import org.palladiosimulator.pcm.repository.PassiveResource;
import org.palladiosimulator.pcm.resourceenvironment.ResourceContainer;
import org.palladiosimulator.pcm.resourcetype.ResourceInterface;
import org.palladiosimulator.pcm.resourcetype.ResourceRepository;
import org.palladiosimulator.pcm.resourcetype.ResourceSignature;
import org.palladiosimulator.pcm.seff.AbstractAction;
import org.palladiosimulator.pcm.seff.ExternalCallAction;
import org.palladiosimulator.pcm.seff.ForkedBehaviour;
import org.palladiosimulator.pcm.seff.InternalAction;
import org.palladiosimulator.pcm.seff.seff_performance.ResourceCall;
import org.palladiosimulator.pcm.seff.seff_performance.SeffPerformanceFactory;
import org.palladiosimulator.pcm.usagemodel.Workload;

/**
 * Hand-made corpus models, each isolating one simulator feature. Generated with {@link PcmBuilder}
 * (deterministic ids) by {@code refsim gen <corpusDir>}.
 */
public final class HandMade {
    private HandMade() {
    }

    /** name -> (features, run.json, builder) */
    static final class Def {
        final String name, features, runJson;
        final Consumer<PcmBuilder> build;
        /** triggersSelfAdaptations of the default monitors (null: all false). */
        java.util.function.BiPredicate<String, String> triggers;
        /** No response-time monitors on external calls (recursion, see Monitors.skipExternalCalls). */
        boolean skipExternalCalls;

        Def(String name, String features, String runJson, Consumer<PcmBuilder> build) {
            this.name = name;
            this.features = features;
            this.runJson = runJson;
            this.build = build;
        }
    }

    static String run(long seed, long maxMeas, long maxTime) {
        return runJson(seed, maxMeas, maxTime, true);
    }

    /** run.json with the network flags always explicit (SimuLizar UI defaults: false / true). */
    public static String runJson(long seed, long maxMeas, long maxTime, boolean linkThroughput) {
        StringBuilder sb = new StringBuilder("{\n  \"seed\": ").append(seed);
        sb.append(",\n  \"max_measurements\": ").append(maxMeas > 0 ? maxMeas : -1);
        sb.append(",\n  \"max_sim_time\": ").append(maxTime > 0 ? maxTime : -1);
        sb.append(",\n  \"simulate_linking_resources\": false");
        sb.append(",\n  \"simulate_throughput_of_linking_resources\": ").append(linkThroughput);
        return sb.append("\n}\n").toString();
    }

    static String runNoThroughput(long seed, long maxMeas) {
        return runJson(seed, maxMeas, 0, false);
    }

    /** One component on one container, provided by the system. */
    static final class Single {
        OperationInterface i;
        OperationSignature op;
        BasicComponent comp;
        OperationProvidedRole prov, sysRole;
        AssemblyContext ac;
        ResourceContainer rc;
    }

    static Single single(PcmBuilder b, Function<BasicComponent, AbstractAction[]> seff, Parameter... params) {
        Single s = new Single();
        s.i = b.iface("IService");
        s.op = b.sig(s.i, "op", params);
        s.comp = b.comp("Server");
        s.prov = b.provides(s.comp, s.i);
        b.seff(s.comp, s.op, seff.apply(s.comp));
        s.ac = b.assembly(s.comp);
        s.sysRole = b.delegateProvided(b.sys, s.i, s.ac, s.prov);
        s.rc = b.container("ServerNode");
        b.allocate(s.ac, s.rc);
        return s;
    }

    static final Map<String, Def> DEFS = new LinkedHashMap<>();

    static void def(String name, String features, String runJson, Consumer<PcmBuilder> build) {
        DEFS.put(name, new Def(name, features, runJson, build));
    }

    static {
        def("h01_ps_single", "open workload Exp interarrival; one internal action with Exp CPU demand on a 1-core PS CPU",
                run(1, 100, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Exp(4.0)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h02_ps_ties", "closed workload, constant demands arriving simultaneously on a PS CPU (tie-breaking of equal remaining demands; rate 2.0)",
                run(1, 30, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "1.0")) });
                    b.resource(s.rc, b.CPU, b.PS, "2.0", 1);
                    b.scenario("Scenario", b.closed(3, "1.0"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h03_fcfs_hdd", "CPU (PS) + HDD (FCFS) demands in one internal action; DoublePDF HDD demand; open workload",
                run(2, 75, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] {
                            b.internal("compute", b.demand(b.CPU, "Exp(5.0)")),
                            b.internal("io", b.demand(b.HDD, "DoublePDF[(0.1;0.3)(0.2;0.5)(0.4;0.2)]")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.resource(s.rc, b.HDD, b.FCFS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(3.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h04_delay_resource", "DELAY resource type with Delay scheduling; UniDouble demand; closed workload with Exp think time",
                run(3, 50, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] {
                            b.internal("wait", b.demand(b.DELAY, "UniDouble(0.2, 0.8)")),
                            b.internal("work", b.demand(b.CPU, "0.05")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.resource(s.rc, b.DELAY, b.DELAY_POLICY, "1.0", 1);
                    b.scenario("Scenario", b.closed(5, "Exp(1.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h05_open_workload", "open workload with usage-level Delay before the call; FCFS CPU; Gamma demand",
                run(4, 75, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Gamma(2.0, 0.1)")) });
                    b.resource(s.rc, b.CPU, b.FCFS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.delay("userDelay", "0.5"),
                            b.elsc("callOp", s.sysRole, s.op));
                });
        def("h06_closed_think", "closed workload (4 users) with Exp think time; Lognorm CPU demand on PS",
                run(5, 60, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Lognorm(-1.5, 0.5)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.closed(4, "Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h07_prob_branch", "SEFF probabilistic BranchAction with 3 transitions of different demands",
                run(6, 100, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.branch("branch",
                            0.2, b.behaviour(b.internal("small", b.demand(b.CPU, "0.01"))),
                            0.5, b.behaviour(b.internal("medium", b.demand(b.CPU, "0.05"))),
                            0.3, b.behaviour(b.internal("large", b.demand(b.CPU, "0.2")))) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(5.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h08_guarded_branch_params", "guarded BranchAction on an input parameter (IntPMF from the usage model); parametric demand n.VALUE * 0.01",
                run(7, 75, 0), b -> {
                    Parameter n = b.param("n", b.INT);
                    Single s = single(b, c -> new AbstractAction[] { b.guarded("guard",
                            "n.VALUE < 3", b.behaviour(b.internal("few", b.demand(b.CPU, "n.VALUE * 0.01"))),
                            "n.VALUE >= 3 AND n.VALUE < 8", b.behaviour(b.internal("some", b.demand(b.CPU, "n.VALUE * 0.02"))),
                            "n.VALUE >= 8", b.behaviour(b.internal("many", b.demand(b.CPU, "0.3")))) }, n);
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(3.0)"), b.elsc("callOp", s.sysRole, s.op,
                            vu("n", VALUE, "IntPMF[(1;0.3)(5;0.5)(10;0.2)]")));
                });
        def("h09_loop", "SEFF LoopAction with IntPMF iteration count; usage-model Loop with constant count",
                run(8, 50, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.loop("loop", "IntPMF[(1;0.2)(2;0.5)(3;0.3)]",
                            b.behaviour(b.internal("body", b.demand(b.CPU, "Exp(20.0)")))) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.closed(2, "0.5"), b.uloop("userLoop", "3", b.elsc("callOp", s.sysRole, s.op)));
                });
        def("h10_collection_iterator", "CollectionIteratorAction over a collection parameter (NUMBER_OF_ELEMENTS and INNER.VALUE from the usage model)",
                run(9, 50, 0), b -> {
                    Parameter items = b.param("items", b.collectionOf("IntList", b.INT));
                    Single s = single(b, c -> new AbstractAction[] { b.iterate("iterate", items,
                            b.behaviour(b.internal("perItem", b.demand(b.CPU, "items.INNER.VALUE * 0.05")))) }, items);
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op,
                            vu("items", NUMBER_OF_ELEMENTS, "IntPMF[(2;0.5)(4;0.5)]"),
                            vu("items.INNER", VALUE, "IntPMF[(1;0.5)(2;0.5)]")));
                });
        def("h11_fork_sync", "ForkAction with 2 synchronous forked behaviours (join via SynchronisationPoint)",
                run(10, 50, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] {
                            b.internal("before", b.demand(b.CPU, "0.01")),
                            b.fork("fork", List.of(), List.of(
                                    b.forked(b.internal("branchA", b.demand(b.CPU, "Exp(10.0)"))),
                                    b.forked(b.internal("branchB", b.demand(b.CPU, "Exp(5.0)"))))),
                            b.internal("after", b.demand(b.CPU, "0.01")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 2);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h12_fork_async", "ForkAction with 2 asynchronous forked behaviours (caller does not wait)",
                run(11, 50, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] {
                            b.fork("fork", List.of(
                                    b.forked(b.internal("asyncA", b.demand(b.CPU, "Exp(8.0)"))),
                                    b.forked(b.internal("asyncB", b.demand(b.CPU, "0.1")))), List.of()),
                            b.internal("main", b.demand(b.CPU, "0.02")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h13_passive_contention", "passive resource (capacity 2) contention: acquire / CPU demand / release with 6 closed users",
                run(12, 75, 0), b -> {
                    PassiveResource[] pr = new PassiveResource[1];
                    Single s = single(b, c -> {
                        pr[0] = b.passive(c, "pool", "2");
                        return new AbstractAction[] { b.internal("pre", b.demand(b.CPU, "0.01")),
                                b.acquire("acquire", pr[0]), b.internal("critical", b.demand(b.CPU, "Exp(4.0)")),
                                b.release("release", pr[0]) };
                    });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 2);
                    b.scenario("Scenario", b.closed(6, "Exp(1.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h14_call_chain_3", "external call chain A -> B -> C across 3 components on 3 containers (with input parameter passing)",
                run(13, 50, 0), b -> {
                    OperationInterface ia = b.iface("IA"), ib = b.iface("IB"), ic = b.iface("IC");
                    Parameter pb = b.param("size", b.INT), pc = b.param("size", b.INT);
                    OperationSignature oa = b.sig(ia, "a"), ob = b.sig(ib, "b", pb), oc = b.sig(ic, "c", pc);
                    BasicComponent ca = b.comp("A"), cb = b.comp("B"), cc = b.comp("C");
                    OperationProvidedRole pa = b.provides(ca, ia), pbr = b.provides(cb, ib), pcr = b.provides(cc, ic);
                    OperationRequiredRole rab = b.requires(ca, ib), rbc = b.requires(cb, ic);
                    b.seff(ca, oa, b.internal("aWork", b.demand(b.CPU, "0.02")),
                            b.call("callB", rab, ob, vu("size", VALUE, "IntPMF[(10;0.5)(20;0.5)]")),
                            b.internal("aPost", b.demand(b.CPU, "0.01")));
                    b.seff(cb, ob, b.internal("bWork", b.demand(b.CPU, "size.VALUE * 0.002")),
                            b.call("callC", rbc, oc, vu("size", VALUE, "size.VALUE * 2")));
                    b.seff(cc, oc, b.internal("cWork", b.demand(b.CPU, "size.VALUE * 0.001")));
                    AssemblyContext aa = b.assembly(ca), ab = b.assembly(cb), ac = b.assembly(cc);
                    b.connect(aa, rab, ab, pbr);
                    b.connect(ab, rbc, ac, pcr);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, ia, aa, pa);
                    ResourceContainer r1 = b.container("NodeA"), r2 = b.container("NodeB"), r3 = b.container("NodeC");
                    b.resource(r1, b.CPU, b.PS, "1.0", 1);
                    b.resource(r2, b.CPU, b.PS, "1.0", 1);
                    b.resource(r3, b.CPU, b.FCFS, "1.0", 1);
                    b.link("LAN", "0.0", "1.0E9", r1, r2, r3);
                    b.allocate(aa, r1);
                    b.allocate(ab, r2);
                    b.allocate(ac, r3);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callA", sys, oa));
                });
        def("h15_composite", "CompositeComponent with two inner basic components (provided + inner assembly connector)",
                run(14, 50, 0), b -> {
                    OperationInterface is = b.iface("IService"), ib = b.iface("IBack");
                    OperationSignature op = b.sig(is, "op"), back = b.sig(ib, "back");
                    BasicComponent front = b.comp("Front"), bk = b.comp("Back");
                    OperationProvidedRole fp = b.provides(front, is), bp = b.provides(bk, ib);
                    OperationRequiredRole fr = b.requires(front, ib);
                    b.seff(front, op, b.internal("frontWork", b.demand(b.CPU, "Exp(20.0)")), b.call("callBack", fr, back));
                    b.seff(bk, back, b.internal("backWork", b.demand(b.CPU, "0.03")));
                    CompositeComponent comp = b.composite("Composite");
                    AssemblyContext af = b.assembly(comp, front, "Inner_Front"), abk = b.assembly(comp, bk, "Inner_Back");
                    b.connect(comp, af, fr, abk, bp);
                    OperationProvidedRole cp = b.delegateProvided(comp, is, af, fp);
                    AssemblyContext ac = b.assembly(comp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, ac, cp);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(ac, rc);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", sys, op));
                });
        def("h16_component_params", "component parameter default (size.VALUE=10) vs assembly-context override (20); same component assembled twice",
                run(15, 50, 0), b -> {
                    OperationInterface is = b.iface("IService"), iw = b.iface("IWorker");
                    OperationSignature op = b.sig(is, "op"), w = b.sig(iw, "work");
                    BasicComponent front = b.comp("Front"), worker = b.comp("Worker");
                    OperationProvidedRole fp = b.provides(front, is), wp = b.provides(worker, iw);
                    OperationRequiredRole r1 = b.requires(front, iw), r2 = b.requires(front, iw);
                    b.componentParameter(worker, vu("size", VALUE, "10"));
                    b.seff(front, op, b.call("callDefault", r1, w), b.call("callOverride", r2, w));
                    b.seff(worker, w, b.internal("work", b.demand(b.CPU, "size.VALUE * 0.005")));
                    AssemblyContext af = b.assembly(front);
                    AssemblyContext a1 = b.assembly(b.sys, worker, "Worker_default");
                    AssemblyContext a2 = b.assembly(b.sys, worker, "Worker_override");
                    b.configParameter(a2, vu("size", VALUE, "20"));
                    b.connect(af, r1, a1, wp);
                    b.connect(af, r2, a2, wp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, af, fp);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(af, rc);
                    b.allocate(a1, rc);
                    b.allocate(a2, rc);
                    b.scenario("Scenario", b.open("Exp(3.0)"), b.elsc("callOp", sys, op));
                });
        def("h17_set_variable", "SetVariableAction (RETURN.VALUE from IntPMF) and ExternalCall return variable usage used in a later demand",
                run(16, 50, 0), b -> {
                    OperationInterface is = b.iface("IService"), iq = b.iface("IQuery");
                    OperationSignature op = b.sig(is, "op"), q = b.sigReturning(iq, "query", b.INT);
                    BasicComponent front = b.comp("Front"), store = b.comp("Store");
                    OperationProvidedRole fp = b.provides(front, is), sp = b.provides(store, iq);
                    OperationRequiredRole fr = b.requires(front, iq);
                    ExternalCallAction call = b.returns(b.call("callQuery", fr, q), vu("result", VALUE, "RETURN.VALUE"));
                    b.seff(front, op, call, b.internal("useResult", b.demand(b.CPU, "result.VALUE * 0.02")));
                    b.seff(store, q, b.internal("lookup", b.demand(b.CPU, "0.01")),
                            b.setVar("setReturn", vu("RETURN", VALUE, "IntPMF[(1;0.5)(3;0.3)(6;0.2)]")));
                    AssemblyContext af = b.assembly(front), as = b.assembly(store);
                    b.connect(af, fr, as, sp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, af, fp);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(af, rc);
                    b.allocate(as, rc);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", sys, op));
                });
        def("h18_infrastructure_call", "InternalAction with an InfrastructureCall (2 calls) to a middleware component via AssemblyInfrastructureConnector",
                run(17, 50, 0), b -> {
                    OperationInterface is = b.iface("IService");
                    OperationSignature op = b.sig(is, "op");
                    InfrastructureInterface ii = b.infraIface("IMiddleware");
                    InfrastructureSignature isig = b.infraSig(ii, "marshal");
                    BasicComponent app = b.comp("App"), mw = b.comp("Middleware");
                    OperationProvidedRole ap = b.provides(app, is);
                    InfrastructureRequiredRole ar = b.requiresInfra(app, ii);
                    InfrastructureProvidedRole mp = b.providesInfra(mw, ii);
                    b.seff(app, op, b.infraCall("withInfra", ar, isig, "2"), b.internal("work", b.demand(b.CPU, "0.05")));
                    b.seff(mw, isig, b.internal("marshalWork", b.demand(b.CPU, "Exp(50.0)")));
                    AssemblyContext aa = b.assembly(app), am = b.assembly(mw);
                    b.connectInfra(b.sys, aa, ar, am, mp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, aa, ap);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(aa, rc);
                    b.allocate(am, rc);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", sys, op));
                });
        Consumer<PcmBuilder> linking = b -> {
            OperationInterface is = b.iface("IService"), ir = b.iface("IRemote");
            Parameter data = b.param("data", b.INT);
            OperationSignature op = b.sig(is, "op"), rop = b.sig(ir, "remote", data);
            BasicComponent client = b.comp("Client"), server = b.comp("RemoteServer");
            OperationProvidedRole cp = b.provides(client, is), sp = b.provides(server, ir);
            OperationRequiredRole cr = b.requires(client, ir);
            b.seff(client, op, b.internal("prepare", b.demand(b.CPU, "0.01")),
                    b.call("callRemote", cr, rop, vu("data", BYTESIZE, "IntPMF[(1000;0.5)(5000;0.5)]")));
            b.seff(server, rop, b.internal("serve", b.demand(b.CPU, "data.BYTESIZE * 0.00001")));
            AssemblyContext ac = b.assembly(client), as = b.assembly(server);
            b.connect(ac, cr, as, sp);
            OperationProvidedRole sys = b.delegateProvided(b.sys, is, ac, cp);
            ResourceContainer c1 = b.container("ClientNode"), c2 = b.container("ServerNode");
            b.resource(c1, b.CPU, b.PS, "1.0", 1);
            b.resource(c2, b.CPU, b.PS, "1.0", 1);
            b.link("LAN", "0.005", "1000000.0", c1, c2);
            b.allocate(ac, c1);
            b.allocate(as, c2);
            b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", sys, op));
        };
        def("h19_linking_resource", "remote call over a LinkingResource (latency 0.005, throughput 1e6) with BYTESIZE parameter; SimuLizar default: throughput of linking resources simulated (sum of BYTESIZE on the call/return frames)",
                run(18, 50, 0), linking);
        def("h19b_linking_no_throughput", "same model as h19 with simulate_throughput_of_linking_resources=false (payload demand 0, latency only)",
                runNoThroughput(18, 50), linking);
        def("h20_ps_multicore", "4-replica PS CPU with open workload (per-core state + overall utilization measurements)",
                run(19, 100, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Exp(1.0)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 4);
                    b.scenario("Scenario", b.open("Exp(3.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h21_stoex_distributions", "StoEx functions and arithmetic: Exp, Norm (via Max), Gamma, GammaMoments, Lognorm, LognormMoments, UniDouble, UniInt, Pois, Binom-free IntPMF/DoublePMF/DoublePDF, Trunc, Round, Ceil, Min/Max, Sqrt, Log",
                run(20, 30, 0), b -> {
                    String[] specs = { "Exp(50.0)", "Max(Norm(0.02, 0.005), 0.0)", "Gamma(2.0, 0.01)",
                            "GammaMoments(0.02, 0.5)", "Lognorm(-4.0, 0.3)", "LognormMoments(0.02, 0.01)",
                            "UniDouble(0.01, 0.03)", "UniInt(1, 3) * 0.01", "Pois(2.0) * 0.005",
                            "IntPMF[(1;0.25)(2;0.75)] * 0.01 + DoublePMF[(0.001;0.5)(0.002;0.5)]",
                            "DoublePDF[(0.01;0.5)(0.02;0.5)]", "Trunc(3.7) * 0.001 + Round(2.5) * 0.001 + Ceil(0.2) * 0.001",
                            "Min(0.05, Exp(40.0)) + Sqrt(0.0001) + Log(10, 1.01)", "(2 + 3) * 0.004 - 0.01 / 2 ^ 2" };
                    Single s = single(b, c -> {
                        AbstractAction[] acts = new AbstractAction[specs.length];
                        for (int i = 0; i < specs.length; i++) {
                            acts[i] = b.internal("d" + i, b.demand(b.CPU, specs[i]));
                        }
                        return acts;
                    });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h22_usage_behaviour", "usage model Delay (Exp), Branch (0.3/0.7 to two operations) and Loop (IntPMF) around entry level system calls",
                run(21, 50, 0), b -> {
                    OperationInterface is = b.iface("IService");
                    OperationSignature read = b.sig(is, "read"), write = b.sig(is, "write");
                    BasicComponent c = b.comp("Server");
                    OperationProvidedRole p = b.provides(c, is);
                    b.seff(c, read, b.internal("read", b.demand(b.CPU, "0.02")));
                    b.seff(c, write, b.internal("write", b.demand(b.CPU, "Exp(10.0)")));
                    AssemblyContext ac = b.assembly(c);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, ac, p);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(ac, rc);
                    b.scenario("Scenario", b.closed(3, "Exp(1.0)"), b.delay("think", "Exp(2.0)"),
                            b.ubranch("choice", 0.3, b.userBehaviour(b.elsc("doRead", sys, read)), 0.7,
                                    b.userBehaviour(b.uloop("writes", "IntPMF[(1;0.5)(2;0.5)]",
                                            b.elsc("doWrite", sys, write)))));
                });
        def("h23_resource_call", "ResourceCall (CPU 'process' resource signature, numberOfCalls = demand) inside an InternalAction",
                run(22, 50, 0), b -> {
                    ResourceRepository rt = (ResourceRepository) b.CPU.eContainer();
                    ResourceInterface cpuIf = rt.getResourceInterfaces__ResourceRepository().get(0);
                    ResourceSignature process = cpuIf.getResourceSignatures__ResourceInterface().get(0);
                    Single s = single(b, c -> {
                        var rr = org.palladiosimulator.pcm.core.entity.EntityFactory.eINSTANCE.createResourceRequiredRole();
                        rr.setId(b.id("rreq"));
                        rr.setEntityName("CpuRequired");
                        rr.setRequiredResourceInterface__ResourceRequiredRole(cpuIf);
                        c.getResourceRequiredRoles__ResourceInterfaceRequiringEntity().add(rr);
                        ResourceCall call = SeffPerformanceFactory.eINSTANCE.createResourceCall();
                        call.setId(b.id("rcall"));
                        call.setSignature__ResourceCall(process);
                        call.setResourceRequiredRole__ResourceCall(rr);
                        call.setNumberOfCalls__ResourceCall(PcmBuilder.rv("Exp(8.0)"));
                        InternalAction ia = b.internal("resourceCall");
                        ia.getResourceCall__Action().add(call);
                        return new AbstractAction[] { ia };
                    });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h24_two_scenarios", "two usage scenarios (open + closed) sharing one FCFS CPU; stop by max sim time",
                run(23, 0, 15), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Exp(10.0)")) });
                    b.resource(s.rc, b.CPU, b.FCFS, "1.0", 1);
                    b.scenario("OpenScenario", b.open("Exp(3.0)"), b.elsc("callOpen", s.sysRole, s.op));
                    b.scenario("ClosedScenario", b.closed(2, "1.5"), b.elsc("callClosed", s.sysRole, s.op));
                });
        def("h25_deterministic_closed", "fully deterministic model (no random draws): closed workload, constant demands on PS + FCFS + DELAY",
                run(24, 30, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("cpu", b.demand(b.CPU, "0.3")),
                            b.internal("disk", b.demand(b.HDD, "0.2")), b.internal("net", b.demand(b.DELAY, "0.1")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.resource(s.rc, b.HDD, b.FCFS, "1.0", 1);
                    b.resource(s.rc, b.DELAY, b.DELAY_POLICY, "1.0", 1);
                    b.scenario("Scenario", b.closed(4, "0.5"), b.elsc("callOp", s.sysRole, s.op));
                });
    }

    static {
        def("h26_fork_double_resume", "sync ForkAction whose 2 children finish without waiting (zero demand): the parent receives two Resume notes (spec SIM-4.4a / ACT-9.5), the 2nd wakes it early from the following CPU demand",
                run(25, 20, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] {
                            b.fork("fork", List.of(), List.of(
                                    b.forked(b.internal("childA", b.demand(b.CPU, "0.0"))),
                                    b.forked(b.internal("childB", b.demand(b.CPU, "0.0"))))),
                            b.internal("afterJoin", b.demand(b.CPU, "0.5")),
                            b.internal("afterJoin2", b.demand(b.CPU, "Exp(10.0)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.closed(2, "Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h27_collection_inner_multi", "CollectionIterator with several random INNER characterisations (VALUE, BYTESIZE) of one collection plus a second collection parameter: per-iteration draw order follows java.util.HashMap iteration of the frame keys (spec ACT-5.6/5.7)",
                run(26, 30, 0), b -> {
                    Parameter items = b.param("items", b.collectionOf("IntList", b.INT));
                    Parameter extra = b.param("extra", b.collectionOf("IntList2", b.INT));
                    Single s = single(b, c -> new AbstractAction[] { b.iterate("iterate", items,
                            b.behaviour(b.internal("perItem", b.demand(b.CPU,
                                    "items.INNER.VALUE * 0.01 + items.INNER.BYTESIZE * 0.0001 + extra.NUMBER_OF_ELEMENTS * 0.001")))) },
                            items, extra);
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op,
                            vu("items", NUMBER_OF_ELEMENTS, "IntPMF[(2;0.5)(3;0.5)]"),
                            vu("items.INNER", VALUE, "IntPMF[(1;0.3)(2;0.3)(5;0.4)]", BYTESIZE, "UniInt(100, 200)"),
                            vu("extra", NUMBER_OF_ELEMENTS, "UniInt(1, 4)"),
                            vu("extra.INNER", VALUE, "Exp(1.0)")));
                });
    }

    static {
        // triggersSelfAdaptations = true (EMF default): the Reconfigurator creates its
        // ReconfigurationProcess at the first runtime-measurement write after t = 0 (MEAS-7.2)
        def("h28_triggers_default", "all monitors with triggersSelfAdaptations=true (the EMF default; attribute omitted): reconfiguration process created when the lazily created passive-resource calculators register at t=0.25 (before the initial state tuple); otherwise measurements as without triggers",
                run(27, 60, 0), b -> {
                    Single s = single(b, c -> {
                        PassiveResource p = b.passive(c, "pool", "2");
                        return new AbstractAction[] { b.acquire("acquire", p),
                                b.internal("work", b.demand(b.CPU, "Exp(8.0)")), b.release("release", p) };
                    });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.scenario("Scenario", b.closed(3, "Exp(2.0)"), b.delay("warmup", "0.25"),
                            b.elsc("callOp", s.sysRole, s.op));
                });
        DEFS.get("h28_triggers_default").triggers = (label, metric) -> true;
        def("h29_triggers_late", "triggersSelfAdaptations=true only on the state of an unused HDD: its only runtime-measurement write after t=0 is the final state tuple at the stop, so the reconfiguration process is created during SimuComModel.finalise (after the stop line)",
                run(28, 0, 20), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Exp(4.0)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    b.resource(s.rc, b.HDD, b.FCFS, "1.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        DEFS.get("h29_triggers_late").triggers = (label, metric) -> label.contains(".HDD ")
                && metric.equals(Monitors.STATE_ACTIVE);
        def("h30_middleware_stream", "simulate_linking_resources=true (middleware marshalling): the payload demand of every assembly-connector call is stream.BYTESIZE of the request frame / result frame (MiddlewareCompletionAwareDemandCalculator), evaluated also for calls within one container; the callees set stream.BYTESIZE of the reply",
                "{\n  \"seed\": 29,\n  \"max_measurements\": 50,\n  \"max_sim_time\": -1,\n  \"simulate_linking_resources\": true,\n  \"simulate_throughput_of_linking_resources\": true\n}\n",
                b -> {
                    OperationInterface is = b.iface("IService"), ir = b.iface("IRemote"), il = b.iface("ILocal");
                    Parameter stream = b.param("stream", b.INT), local = b.param("stream", b.INT);
                    OperationSignature op = b.sig(is, "op"), rop = b.sig(ir, "remote", stream),
                            lop = b.sig(il, "local", local);
                    BasicComponent client = b.comp("Client"), server = b.comp("RemoteServer"),
                            helper = b.comp("LocalHelper");
                    OperationProvidedRole cp = b.provides(client, is), sp = b.provides(server, ir),
                            hp = b.provides(helper, il);
                    OperationRequiredRole cr = b.requires(client, ir), ch = b.requires(client, il);
                    b.seff(client, op, b.internal("prepare", b.demand(b.CPU, "0.01")),
                            b.call("callLocal", ch, lop, vu("stream", BYTESIZE, "UniInt(10, 20)")),
                            b.call("callRemote", cr, rop, vu("stream", BYTESIZE, "IntPMF[(1000;0.5)(5000;0.5)]")));
                    b.seff(server, rop, b.internal("serve", b.demand(b.CPU, "stream.BYTESIZE * 0.00001")),
                            b.setVar("reply", vu("stream", BYTESIZE, "stream.BYTESIZE * 2")));
                    b.seff(helper, lop, b.internal("help", b.demand(b.CPU, "0.002")),
                            b.setVar("reply", vu("stream", BYTESIZE, "3")));
                    AssemblyContext ac = b.assembly(client), as = b.assembly(server), ah = b.assembly(helper);
                    b.connect(ac, cr, as, sp);
                    b.connect(ac, ch, ah, hp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, ac, cp);
                    ResourceContainer c1 = b.container("ClientNode"), c2 = b.container("ServerNode");
                    b.resource(c1, b.CPU, b.PS, "1.0", 1);
                    b.resource(c2, b.CPU, b.PS, "1.0", 1);
                    b.link("LAN", "0.005", "1000000.0", c1, c2);
                    b.allocate(ac, c1);
                    b.allocate(ah, c1);
                    b.allocate(as, c2);
                    b.scenario("Scenario", b.open("Exp(4.0)"), b.elsc("callOp", sys, op));
                });
        def("h31_nested_container", "a resource container nested in the allocation target (with its own CPU and default monitors) and a nested container of the nested one: SimuLizar 5.2.2 creates simulated containers for top-level containers only, so the nested ones are ignored",
                run(30, 50, 0), b -> {
                    Single s = single(b, c -> new AbstractAction[] { b.internal("work", b.demand(b.CPU, "Exp(4.0)")) });
                    b.resource(s.rc, b.CPU, b.PS, "1.0", 1);
                    ResourceContainer n1 = b.nestedContainer(s.rc, "Blade");
                    b.resource(n1, b.CPU, b.PS, "2.0", 2);
                    ResourceContainer n2 = b.nestedContainer(n1, "Socket");
                    b.resource(n2, b.CPU, b.FCFS, "3.0", 1);
                    b.scenario("Scenario", b.open("Exp(2.0)"), b.elsc("callOp", s.sysRole, s.op));
                });
        def("h32_recursion", "recursive calls: two assemblies of one component call each other (probabilistic branch, depth unbounded but finite); no response-time monitors on the external call (with them SimuLizar aborts: 'First measurement to the same context arrived')",
                run(31, 60, 0), b -> {
                    OperationInterface is = b.iface("IRec");
                    OperationSignature op = b.sig(is, "op");
                    BasicComponent rec = b.comp("Rec");
                    OperationProvidedRole rp = b.provides(rec, is);
                    OperationRequiredRole rr = b.requires(rec, is);
                    b.seff(rec, op, b.internal("work", b.demand(b.CPU, "Exp(20.0)")),
                            b.branch("recurse", 0.4, b.behaviour(b.call("callOther", rr, op)), 0.6,
                                    b.behaviour(b.internal("leaf", b.demand(b.CPU, "0.01")))));
                    AssemblyContext a1 = b.assembly(b.sys, rec, "Assembly_Rec1"), a2 = b.assembly(b.sys, rec, "Assembly_Rec2");
                    b.connect(a1, rr, a2, rp);
                    b.connect(a2, rr, a1, rp);
                    OperationProvidedRole sys = b.delegateProvided(b.sys, is, a1, rp);
                    ResourceContainer rc = b.container("Node");
                    b.resource(rc, b.CPU, b.PS, "1.0", 1);
                    b.allocate(a1, rc);
                    b.allocate(a2, rc);
                    b.scenario("Scenario", b.open("Exp(3.0)"), b.elsc("callOp", sys, op));
                });
        DEFS.get("h32_recursion").skipExternalCalls = true;
    }

    public static int generate(String[] a) throws Exception {
        File root = new File(a[0]).getCanonicalFile();
        Set<String> only = null;
        for (int i = 1; i < a.length; i++) {
            if (a[i].equals("--only")) {
                only = new HashSet<>(Arrays.asList(a[++i].split(",")));
            }
        }
        List<String> done = new ArrayList<>();
        for (Def d : DEFS.values()) {
            if (only != null && !only.contains(d.name)) {
                continue;
            }
            File dir = new File(root, d.name);
            // remove previously generated model files (keep expected/)
            File[] old = dir.listFiles();
            if (old != null) {
                for (File f : old) {
                    if (f.isFile()) {
                        f.delete();
                    }
                }
            }
            PcmBuilder b = new PcmBuilder(d.name);
            d.build.accept(b);
            Monitors.triggers = d.triggers;
            Monitors.skipExternalCalls = d.skipExternalCalls;
            try {
                b.save(dir);
            } finally {
                Monitors.triggers = null;
                Monitors.skipExternalCalls = false;
            }
            Files.writeString(new File(dir, "run.json").toPath(), d.runJson);
            Files.writeString(new File(dir, "FEATURES.txt").toPath(), d.features + "\n");
            done.add(d.name);
        }
        System.err.println("[refsim] generated " + done.size() + " models: " + done);
        return 0;
    }
}
