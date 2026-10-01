package refsim.corpus;

import java.io.File;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.emf.common.util.TreeIterator;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.resource.ResourceSet;
import org.eclipse.emf.ecore.xmi.XMLResource;
import org.palladiosimulator.edp2.models.measuringpoint.MeasuringPoint;
import org.palladiosimulator.edp2.models.measuringpoint.MeasuringPointRepository;
import org.palladiosimulator.edp2.models.measuringpoint.MeasuringpointFactory;
import org.palladiosimulator.metricspec.MetricDescription;
import org.palladiosimulator.monitorrepository.MeasurementSpecification;
import org.palladiosimulator.monitorrepository.Monitor;
import org.palladiosimulator.monitorrepository.MonitorRepository;
import org.palladiosimulator.monitorrepository.MonitorRepositoryFactory;
import org.palladiosimulator.pcm.allocation.Allocation;
import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.core.composition.ComposedStructure;
import org.palladiosimulator.pcm.repository.BasicComponent;
import org.palladiosimulator.pcm.repository.OperationProvidedRole;
import org.palladiosimulator.pcm.repository.OperationSignature;
import org.palladiosimulator.pcm.repository.PassiveResource;
import org.palladiosimulator.pcm.repository.ProvidedRole;
import org.palladiosimulator.pcm.repository.RepositoryComponent;
import org.palladiosimulator.pcm.resourceenvironment.ProcessingResourceSpecification;
import org.palladiosimulator.pcm.resourceenvironment.ResourceContainer;
import org.palladiosimulator.pcm.seff.ExternalCallAction;
import org.palladiosimulator.pcm.seff.ServiceEffectSpecification;
import org.palladiosimulator.pcm.usagemodel.EntryLevelSystemCall;
import org.palladiosimulator.pcm.usagemodel.UsageModel;
import org.palladiosimulator.pcm.usagemodel.UsageScenario;
import org.palladiosimulator.pcmmeasuringpoint.PcmmeasuringpointFactory;

/**
 * Generates the refsim default monitor repository for a model: FeedThrough monitors for
 * <ul>
 * <li>Response Time: every usage scenario, entry level system call, system provided operation and
 * external call action (of components instantiated in the system);</li>
 * <li>State of Active Resource (one per replica) and Resource Demand (replica 0): every processing
 * resource;</li>
 * <li>Waiting Time, Holding Time, State of Passive Resource: every passive resource per assembly
 * context.</li>
 * </ul>
 * Monitor ids are derived from the model name and a counter (deterministic).
 */
public final class Monitors {
    public static final Map<Object, Object> SAVE_OPTIONS = new HashMap<>();
    static {
        SAVE_OPTIONS.put(XMLResource.OPTION_ENCODING, "UTF-8");
        SAVE_OPTIONS.put(XMLResource.OPTION_SAVE_TYPE_INFORMATION, Boolean.FALSE);
        SAVE_OPTIONS.put(XMLResource.OPTION_LINE_WIDTH, 120);
        // always write hrefs between model files as relative URIs ("x.repository#_id")
        SAVE_OPTIONS.put(XMLResource.OPTION_URI_HANDLER, new org.eclipse.emf.ecore.xmi.impl.URIHandlerImpl() {
            @Override
            public URI deresolve(URI uri) {
                if (uri.isFile() && baseURI != null && baseURI.isFile()) {
                    return uri.deresolve(baseURI, true, true, false);
                }
                return super.deresolve(uri);
            }
        });
    }

    static final String METRICS = "pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec#";
    static final String RESPONSE_TIME = "_6rYmYs7nEeOX_4BzImuHbA";
    static final String STATE_ACTIVE = "_paDhIs7qEeOX_4BzImuHbA";
    static final String RESOURCE_DEMAND = "_eg_F0s7qEeOX_4BzImuHbA";
    static final String WAITING_TIME = "_QWjAYs7qEeOX_4BzImuHbA";
    static final String HOLDING_TIME = "_zETOUs7pEeOX_4BzImuHbA";
    static final String STATE_PASSIVE = "_x0-pks7rEeOX_4BzImuHbA";

    /** Import option: no Response Time monitors on external calls (recursive calls make SimuLizar's
     * response time calculator fail: "First measurement to the same context arrived ..."). */
    public static boolean skipExternalCalls;

    /** triggersSelfAdaptations per (monitor label, metric id); null: false everywhere (refsim default).
     * true is the EMF default, i.e. what user-made monitor repositories usually contain. */
    public static java.util.function.BiPredicate<String, String> triggers;

    private Monitors() {
    }

    static final class Ctx {
        final ResourceSet rs;
        final MeasuringPointRepository mpr;
        final MonitorRepository mon;
        final String name;
        int n;

        Ctx(ResourceSet rs, MeasuringPointRepository mpr, MonitorRepository mon, String name) {
            this.rs = rs;
            this.mpr = mpr;
            this.mon = mon;
            this.name = name;
        }

        void add(MeasuringPoint mp, String label, String... metricIds) {
            mpr.getMeasuringPoints().add(mp);
            Monitor m = MonitorRepositoryFactory.eINSTANCE.createMonitor();
            m.setId("_" + name + "_mon" + (++n));
            m.setEntityName(label);
            m.setActivated(true);
            m.setMeasuringPoint(mp);
            for (String id : metricIds) {
                MeasurementSpecification s = MonitorRepositoryFactory.eINSTANCE.createMeasurementSpecification();
                s.setId("_" + name + "_ms" + n + "_" + id.substring(1, 5));
                s.setMetricDescription((MetricDescription) rs.getEObject(URI.createURI(METRICS + id), true));
                // refsim default: no reconfiguration triggers; see `triggers`
                s.setTriggersSelfAdaptations(triggers != null && triggers.test(label, id));
                var ft = MonitorRepositoryFactory.eINSTANCE.createFeedThrough();
                ft.setId("_" + name + "_ft" + n + "_" + id.substring(1, 5));
                s.setProcessingType(ft);
                m.getMeasurementSpecifications().add(s);
            }
            mon.getMonitors().add(m);
        }
    }

    /**
     * Creates &lt;dir&gt;/&lt;name&gt;.measuringpoint and &lt;dir&gt;/&lt;name&gt;.monitorrepository in the resource
     * set (not saved).
     */
    public static MonitorRepository addDefaultMonitors(ResourceSet rs, UsageModel um, Allocation alloc, File dir,
            String name) {
        MeasuringPointRepository mpr = MeasuringpointFactory.eINSTANCE.createMeasuringPointRepository();
        mpr.setId("_" + name + "_mpr");
        MonitorRepository mon = MonitorRepositoryFactory.eINSTANCE.createMonitorRepository();
        mon.setId("_" + name + "_monrepo");
        mon.setEntityName("refsim default monitors");
        Resource r1 = rs.createResource(URI.createFileURI(new File(dir, name + ".measuringpoint").getAbsolutePath()));
        r1.getContents().add(mpr);
        Resource r2 = rs
            .createResource(URI.createFileURI(new File(dir, name + ".monitorrepository").getAbsolutePath()));
        r2.getContents().add(mon);
        Ctx c = new Ctx(rs, mpr, mon, name);
        PcmmeasuringpointFactory F = PcmmeasuringpointFactory.eINSTANCE;

        for (UsageScenario us : um.getUsageScenario_UsageModel()) {
            var mp = F.createUsageScenarioMeasuringPoint();
            mp.setUsageScenario(us);
            c.add(mp, "RT scenario " + us.getEntityName(), RESPONSE_TIME);
        }
        for (TreeIterator<EObject> it = um.eAllContents(); it.hasNext();) {
            EObject o = it.next();
            if (o instanceof EntryLevelSystemCall) {
                var mp = F.createEntryLevelSystemCallMeasuringPoint();
                mp.setEntryLevelSystemCall((EntryLevelSystemCall) o);
                c.add(mp, "RT call " + ((EntryLevelSystemCall) o).getEntityName(), RESPONSE_TIME);
            }
        }
        org.palladiosimulator.pcm.system.System sys = alloc.getSystem_Allocation();
        for (ProvidedRole pr : sys.getProvidedRoles_InterfaceProvidingEntity()) {
            if (pr instanceof OperationProvidedRole) {
                for (OperationSignature s : ((OperationProvidedRole) pr).getProvidedInterface__OperationProvidedRole()
                    .getSignatures__OperationInterface()) {
                    var mp = F.createSystemOperationMeasuringPoint();
                    mp.setSystem(sys);
                    mp.setRole(pr);
                    mp.setOperationSignature(s);
                    c.add(mp, "RT system op " + s.getEntityName(), RESPONSE_TIME);
                }
            }
        }
        // components instantiated in the system (recursively through composites), in model order
        Set<RepositoryComponent> comps = new LinkedHashSet<>();
        List<AssemblyContext[]> passiveOwners = new ArrayList<>();
        collect(sys, comps, passiveOwners);
        Set<ExternalCallAction> calls = new LinkedHashSet<>();
        for (RepositoryComponent rc : comps) {
            if (rc instanceof BasicComponent) {
                for (ServiceEffectSpecification seff : ((BasicComponent) rc)
                    .getServiceEffectSpecifications__BasicComponent()) {
                    for (TreeIterator<EObject> it = seff.eAllContents(); it.hasNext();) {
                        EObject o = it.next();
                        if (o instanceof ExternalCallAction) {
                            calls.add((ExternalCallAction) o);
                        }
                    }
                }
            }
        }
        for (ExternalCallAction a : skipExternalCalls ? java.util.List.<ExternalCallAction> of() : calls) {
            var mp = F.createExternalCallActionMeasuringPoint();
            mp.setExternalCall(a);
            c.add(mp, "RT external call " + a.getEntityName(), RESPONSE_TIME);
        }
        for (ResourceContainer rc : alloc.getTargetResourceEnvironment_Allocation()
            .getResourceContainer_ResourceEnvironment()) {
            addContainer(c, F, rc);
        }
        for (AssemblyContext[] ac : passiveOwners) {
            BasicComponent bc = (BasicComponent) ac[0].getEncapsulatedComponent__AssemblyContext();
            for (PassiveResource p : bc.getPassiveResource_BasicComponent()) {
                var mp = F.createAssemblyPassiveResourceMeasuringPoint();
                mp.setAssembly(ac[0]);
                mp.setPassiveResource(p);
                c.add(mp, "passive " + ac[0].getEntityName() + "." + p.getEntityName(), WAITING_TIME, HOLDING_TIME,
                        STATE_PASSIVE);
            }
        }
        return mon;
    }

    private static void addContainer(Ctx c, PcmmeasuringpointFactory F, ResourceContainer rc) {
        for (ProcessingResourceSpecification prs : rc.getActiveResourceSpecifications_ResourceContainer()) {
            for (int i = 0; i < Math.max(1, prs.getNumberOfReplicas()); i++) {
                var mp = F.createActiveResourceMeasuringPoint();
                mp.setActiveResource(prs);
                mp.setReplicaID(i);
                if (i == 0) {
                    c.add(mp, "resource " + rc.getEntityName() + "."
                            + prs.getActiveResourceType_ActiveResourceSpecification().getEntityName() + " #" + i,
                            STATE_ACTIVE, RESOURCE_DEMAND);
                } else {
                    c.add(mp, "resource " + rc.getEntityName() + "."
                            + prs.getActiveResourceType_ActiveResourceSpecification().getEntityName() + " #" + i,
                            STATE_ACTIVE);
                }
            }
        }
        for (ResourceContainer nested : rc.getNestedResourceContainers__ResourceContainer()) {
            addContainer(c, F, nested);
        }
    }

    private static void collect(ComposedStructure cs, Set<RepositoryComponent> comps, List<AssemblyContext[]> passive) {
        for (AssemblyContext ac : cs.getAssemblyContexts__ComposedStructure()) {
            RepositoryComponent rc = ac.getEncapsulatedComponent__AssemblyContext();
            comps.add(rc);
            if (rc instanceof BasicComponent && !((BasicComponent) rc).getPassiveResource_BasicComponent().isEmpty()) {
                passive.add(new AssemblyContext[] { ac });
            }
            if (rc instanceof ComposedStructure) {
                collect((ComposedStructure) rc, comps, passive);
            }
        }
    }
}
