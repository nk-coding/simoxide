package refsim.corpus;

import java.io.File;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.resource.ResourceSet;
import org.eclipse.emf.ecore.resource.impl.ResourceSetImpl;
import org.palladiosimulator.pcm.allocation.Allocation;
import org.palladiosimulator.pcm.allocation.AllocationContext;
import org.palladiosimulator.pcm.allocation.AllocationFactory;
import org.palladiosimulator.pcm.core.CoreFactory;
import org.palladiosimulator.pcm.core.PCMRandomVariable;
import org.palladiosimulator.pcm.core.composition.AssemblyConnector;
import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.core.composition.AssemblyInfrastructureConnector;
import org.palladiosimulator.pcm.core.composition.ComposedStructure;
import org.palladiosimulator.pcm.core.composition.CompositionFactory;
import org.palladiosimulator.pcm.core.composition.ProvidedDelegationConnector;
import org.palladiosimulator.pcm.core.composition.RequiredDelegationConnector;
import org.palladiosimulator.pcm.core.entity.Entity;
import org.palladiosimulator.pcm.core.entity.InterfaceProvidingRequiringEntity;
import org.palladiosimulator.pcm.parameter.ParameterFactory;
import org.palladiosimulator.pcm.parameter.VariableCharacterisation;
import org.palladiosimulator.pcm.parameter.VariableCharacterisationType;
import org.palladiosimulator.pcm.parameter.VariableUsage;
import org.palladiosimulator.pcm.repository.BasicComponent;
import org.palladiosimulator.pcm.repository.CollectionDataType;
import org.palladiosimulator.pcm.repository.CompositeComponent;
import org.palladiosimulator.pcm.repository.DataType;
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
import org.palladiosimulator.pcm.repository.PrimitiveDataType;
import org.palladiosimulator.pcm.repository.Repository;
import org.palladiosimulator.pcm.repository.RepositoryComponent;
import org.palladiosimulator.pcm.repository.RepositoryFactory;
import org.palladiosimulator.pcm.resourceenvironment.CommunicationLinkResourceSpecification;
import org.palladiosimulator.pcm.resourceenvironment.HDDProcessingResourceSpecification;
import org.palladiosimulator.pcm.resourceenvironment.LinkingResource;
import org.palladiosimulator.pcm.resourceenvironment.ProcessingResourceSpecification;
import org.palladiosimulator.pcm.resourceenvironment.ResourceContainer;
import org.palladiosimulator.pcm.resourceenvironment.ResourceEnvironment;
import org.palladiosimulator.pcm.resourceenvironment.ResourceenvironmentFactory;
import org.palladiosimulator.pcm.resourcetype.CommunicationLinkResourceType;
import org.palladiosimulator.pcm.resourcetype.ProcessingResourceType;
import org.palladiosimulator.pcm.resourcetype.ResourceRepository;
import org.palladiosimulator.pcm.resourcetype.ResourceType;
import org.palladiosimulator.pcm.resourcetype.SchedulingPolicy;
import org.palladiosimulator.pcm.seff.AbstractAction;
import org.palladiosimulator.pcm.seff.AcquireAction;
import org.palladiosimulator.pcm.seff.BranchAction;
import org.palladiosimulator.pcm.seff.CollectionIteratorAction;
import org.palladiosimulator.pcm.seff.ExternalCallAction;
import org.palladiosimulator.pcm.seff.ForkAction;
import org.palladiosimulator.pcm.seff.ForkedBehaviour;
import org.palladiosimulator.pcm.seff.GuardedBranchTransition;
import org.palladiosimulator.pcm.seff.InternalAction;
import org.palladiosimulator.pcm.seff.LoopAction;
import org.palladiosimulator.pcm.seff.ProbabilisticBranchTransition;
import org.palladiosimulator.pcm.seff.ReleaseAction;
import org.palladiosimulator.pcm.seff.ResourceDemandingBehaviour;
import org.palladiosimulator.pcm.seff.ResourceDemandingSEFF;
import org.palladiosimulator.pcm.seff.SeffFactory;
import org.palladiosimulator.pcm.seff.SetVariableAction;
import org.palladiosimulator.pcm.seff.SynchronisationPoint;
import org.palladiosimulator.pcm.seff.seff_performance.InfrastructureCall;
import org.palladiosimulator.pcm.seff.seff_performance.ParametricResourceDemand;
import org.palladiosimulator.pcm.seff.seff_performance.SeffPerformanceFactory;
import org.palladiosimulator.pcm.system.System;
import org.palladiosimulator.pcm.system.SystemFactory;
import org.palladiosimulator.pcm.usagemodel.AbstractUserAction;
import org.palladiosimulator.pcm.usagemodel.Branch;
import org.palladiosimulator.pcm.usagemodel.BranchTransition;
import org.palladiosimulator.pcm.usagemodel.ClosedWorkload;
import org.palladiosimulator.pcm.usagemodel.Delay;
import org.palladiosimulator.pcm.usagemodel.EntryLevelSystemCall;
import org.palladiosimulator.pcm.usagemodel.Loop;
import org.palladiosimulator.pcm.usagemodel.OpenWorkload;
import org.palladiosimulator.pcm.usagemodel.ScenarioBehaviour;
import org.palladiosimulator.pcm.usagemodel.UsageModel;
import org.palladiosimulator.pcm.usagemodel.UsageScenario;
import org.palladiosimulator.pcm.usagemodel.UsagemodelFactory;
import org.palladiosimulator.pcm.usagemodel.Workload;

import de.uka.ipd.sdq.identifier.Identifier;
import de.uka.ipd.sdq.stoex.AbstractNamedReference;
import de.uka.ipd.sdq.stoex.NamespaceReference;
import de.uka.ipd.sdq.stoex.StoexFactory;
import de.uka.ipd.sdq.stoex.VariableReference;

/**
 * Small EMF builder for hand-made PCM corpus models. All ids are deterministic
 * ("_&lt;model&gt;_&lt;kind&gt;&lt;n&gt;"), so regenerating a model yields identical XMI.
 */
public final class PcmBuilder {
    static final RepositoryFactory R = RepositoryFactory.eINSTANCE;
    static final SeffFactory S = SeffFactory.eINSTANCE;
    static final SeffPerformanceFactory SP = SeffPerformanceFactory.eINSTANCE;
    static final CompositionFactory C = CompositionFactory.eINSTANCE;
    static final UsagemodelFactory U = UsagemodelFactory.eINSTANCE;
    static final ResourceenvironmentFactory RE = ResourceenvironmentFactory.eINSTANCE;

    public final String name;
    public final ResourceSet rs = new ResourceSetImpl();
    public final Repository repo;
    public final System sys;
    public final ResourceEnvironment env;
    public final Allocation alloc;
    public final UsageModel um;

    public final ProcessingResourceType CPU, HDD, DELAY;
    public final CommunicationLinkResourceType LAN;
    public final SchedulingPolicy PS, FCFS, DELAY_POLICY;
    public final PrimitiveDataType INT, DOUBLE, BOOL, STRING;

    private final Map<String, Integer> counters = new HashMap<>();

    public PcmBuilder(String name) {
        this.name = name;
        ResourceRepository rt = (ResourceRepository) rs
            .getResource(URI.createURI("pathmap://PCM_MODELS/Palladio.resourcetype"), true)
            .getContents()
            .get(0);
        CPU = (ProcessingResourceType) type(rt, "CPU");
        HDD = (ProcessingResourceType) type(rt, "HDD");
        DELAY = (ProcessingResourceType) type(rt, "DELAY");
        LAN = (CommunicationLinkResourceType) type(rt, "LAN");
        PS = policy(rt, "ProcessorSharing");
        FCFS = policy(rt, "FCFS");
        DELAY_POLICY = policy(rt, "Delay");
        Repository prim = (Repository) rs
            .getResource(URI.createURI("pathmap://PCM_MODELS/PrimitiveTypes.repository"), true)
            .getContents()
            .get(0);
        INT = prim(prim, "INT");
        DOUBLE = prim(prim, "DOUBLE");
        BOOL = prim(prim, "BOOL");
        STRING = prim(prim, "STRING");

        repo = R.createRepository();
        set(repo, "repository", name);
        sys = SystemFactory.eINSTANCE.createSystem();
        set(sys, "system", name);
        env = RE.createResourceEnvironment();
        env.setEntityName(name);
        alloc = AllocationFactory.eINSTANCE.createAllocation();
        set(alloc, "allocation", name);
        alloc.setSystem_Allocation(sys);
        alloc.setTargetResourceEnvironment_Allocation(env);
        um = U.createUsageModel();
    }

    private static ResourceType type(ResourceRepository rt, String n) {
        for (ResourceType t : rt.getAvailableResourceTypes_ResourceRepository()) {
            if (n.equals(t.getEntityName())) {
                return t;
            }
        }
        throw new IllegalStateException(n);
    }

    private static SchedulingPolicy policy(ResourceRepository rt, String id) {
        for (SchedulingPolicy p : rt.getSchedulingPolicies__ResourceRepository()) {
            if (id.equals(p.getId())) {
                return p;
            }
        }
        throw new IllegalStateException(id);
    }

    private static PrimitiveDataType prim(Repository r, String n) {
        for (DataType d : r.getDataTypes__Repository()) {
            if (d instanceof PrimitiveDataType && n.equals(((PrimitiveDataType) d).getType().getName())) {
                return (PrimitiveDataType) d;
            }
        }
        throw new IllegalStateException(n);
    }

    // ------------------------------------------------------------------ ids / names

    public String id(String kind) {
        int n = counters.merge(kind, 1, Integer::sum);
        return "_" + name + "_" + kind + n;
    }

    public <T extends Identifier> T set(T o, String kind, String entityName) {
        o.setId(id(kind));
        if (entityName != null && o instanceof org.palladiosimulator.pcm.core.entity.NamedElement) {
            ((org.palladiosimulator.pcm.core.entity.NamedElement) o).setEntityName(entityName);
        }
        return o;
    }

    public static PCMRandomVariable rv(String spec) {
        PCMRandomVariable v = CoreFactory.eINSTANCE.createPCMRandomVariable();
        v.setSpecification(spec);
        return v;
    }

    // ------------------------------------------------------------------ repository

    public OperationInterface iface(String n) {
        OperationInterface i = set(R.createOperationInterface(), "if", n);
        repo.getInterfaces__Repository().add(i);
        return i;
    }

    public OperationSignature sig(OperationInterface i, String n, Parameter... params) {
        OperationSignature s = set(R.createOperationSignature(), "sig", n);
        i.getSignatures__OperationInterface().add(s);
        for (Parameter p : params) {
            s.getParameters__OperationSignature().add(p);
        }
        return s;
    }

    public OperationSignature sigReturning(OperationInterface i, String n, DataType ret, Parameter... params) {
        OperationSignature s = sig(i, n, params);
        s.setReturnType__OperationSignature(ret);
        return s;
    }

    public Parameter param(String n, DataType t) {
        Parameter p = R.createParameter();
        p.setParameterName(n);
        p.setDataType__Parameter(t);
        return p;
    }

    public CollectionDataType collectionOf(String n, DataType inner) {
        CollectionDataType c = set(R.createCollectionDataType(), "dt", n);
        c.setInnerType_CollectionDataType(inner);
        repo.getDataTypes__Repository().add(c);
        return c;
    }

    public BasicComponent comp(String n) {
        BasicComponent c = set(R.createBasicComponent(), "comp", n);
        repo.getComponents__Repository().add(c);
        return c;
    }

    public CompositeComponent composite(String n) {
        CompositeComponent c = set(R.createCompositeComponent(), "comp", n);
        repo.getComponents__Repository().add(c);
        return c;
    }

    public OperationProvidedRole provides(InterfaceProvidingRequiringEntity c, OperationInterface i) {
        OperationProvidedRole r = set(R.createOperationProvidedRole(), "prov", "Provided_" + i.getEntityName());
        r.setProvidedInterface__OperationProvidedRole(i);
        c.getProvidedRoles_InterfaceProvidingEntity().add(r);
        return r;
    }

    public OperationRequiredRole requires(InterfaceProvidingRequiringEntity c, OperationInterface i) {
        OperationRequiredRole r = set(R.createOperationRequiredRole(), "req", "Required_" + i.getEntityName());
        r.setRequiredInterface__OperationRequiredRole(i);
        c.getRequiredRoles_InterfaceRequiringEntity().add(r);
        return r;
    }

    public InfrastructureInterface infraIface(String n) {
        InfrastructureInterface i = set(R.createInfrastructureInterface(), "if", n);
        repo.getInterfaces__Repository().add(i);
        return i;
    }

    public InfrastructureSignature infraSig(InfrastructureInterface i, String n) {
        InfrastructureSignature s = set(R.createInfrastructureSignature(), "sig", n);
        i.getInfrastructureSignatures__InfrastructureInterface().add(s);
        return s;
    }

    public InfrastructureProvidedRole providesInfra(InterfaceProvidingRequiringEntity c, InfrastructureInterface i) {
        InfrastructureProvidedRole r = set(R.createInfrastructureProvidedRole(), "prov", "Provided_" + i.getEntityName());
        r.setProvidedInterface__InfrastructureProvidedRole(i);
        c.getProvidedRoles_InterfaceProvidingEntity().add(r);
        return r;
    }

    public InfrastructureRequiredRole requiresInfra(InterfaceProvidingRequiringEntity c, InfrastructureInterface i) {
        InfrastructureRequiredRole r = set(R.createInfrastructureRequiredRole(), "req", "Required_" + i.getEntityName());
        r.setRequiredInterface__InfrastructureRequiredRole(i);
        c.getRequiredRoles_InterfaceRequiringEntity().add(r);
        return r;
    }

    public PassiveResource passive(BasicComponent c, String n, String capacity) {
        PassiveResource p = set(R.createPassiveResource(), "pr", n);
        p.setCapacity_PassiveResource(rv(capacity));
        c.getPassiveResource_BasicComponent().add(p);
        return p;
    }

    /** Component parameter default (BasicComponent.componentParameterUsage). */
    public void componentParameter(BasicComponent c, VariableUsage vu) {
        c.getComponentParameterUsage_ImplementationComponentType().add(vu);
    }

    // ------------------------------------------------------------------ SEFF

    public ResourceDemandingSEFF seff(BasicComponent c, org.palladiosimulator.pcm.repository.Signature s,
            AbstractAction... actions) {
        ResourceDemandingSEFF seff = S.createResourceDemandingSEFF();
        seff.setId(id("seff"));
        seff.setDescribedService__SEFF(s);
        c.getServiceEffectSpecifications__BasicComponent().add(seff);
        chain(seff, actions);
        return seff;
    }

    /** Links start -> actions... -> stop inside the behaviour. */
    public <B extends ResourceDemandingBehaviour> B chain(B b, AbstractAction... actions) {
        AbstractAction start = set(S.createStartAction(), "act", "start");
        AbstractAction stop = set(S.createStopAction(), "act", "stop");
        b.getSteps_Behaviour().add(start);
        AbstractAction prev = start;
        for (AbstractAction a : actions) {
            b.getSteps_Behaviour().add(a);
            a.setPredecessor_AbstractAction(prev);
            prev = a;
        }
        b.getSteps_Behaviour().add(stop);
        stop.setPredecessor_AbstractAction(prev);
        return b;
    }

    public ResourceDemandingBehaviour behaviour(AbstractAction... actions) {
        ResourceDemandingBehaviour b = S.createResourceDemandingBehaviour();
        b.setId(id("rdb"));
        return chain(b, actions);
    }

    public ParametricResourceDemand demand(ProcessingResourceType t, String spec) {
        ParametricResourceDemand d = SP.createParametricResourceDemand();
        d.setRequiredResource_ParametricResourceDemand(t);
        d.setSpecification_ParametericResourceDemand(rv(spec));
        return d;
    }

    public InternalAction internal(String n, ParametricResourceDemand... demands) {
        InternalAction a = set(S.createInternalAction(), "act", n);
        for (ParametricResourceDemand d : demands) {
            a.getResourceDemand_Action().add(d);
        }
        return a;
    }

    public InternalAction infraCall(String n, InfrastructureRequiredRole role, InfrastructureSignature s,
            String calls, VariableUsage... inputs) {
        InternalAction a = set(S.createInternalAction(), "act", n);
        InfrastructureCall ic = SP.createInfrastructureCall();
        ic.setId(id("ic"));
        ic.setRequiredRole__InfrastructureCall(role);
        ic.setSignature__InfrastructureCall(s);
        ic.setNumberOfCalls__InfrastructureCall(rv(calls));
        for (VariableUsage v : inputs) {
            ic.getInputVariableUsages__CallAction().add(v);
        }
        a.getInfrastructureCall__Action().add(ic);
        return a;
    }

    public ExternalCallAction call(String n, OperationRequiredRole role, OperationSignature s, VariableUsage... inputs) {
        ExternalCallAction a = set(S.createExternalCallAction(), "act", n);
        a.setRole_ExternalService(role);
        a.setCalledService_ExternalService(s);
        for (VariableUsage v : inputs) {
            a.getInputVariableUsages__CallAction().add(v);
        }
        return a;
    }

    public ExternalCallAction returns(ExternalCallAction a, VariableUsage... outs) {
        for (VariableUsage v : outs) {
            a.getReturnVariableUsage__CallReturnAction().add(v);
        }
        return a;
    }

    /** Probabilistic branch: pairs of (Double probability, ResourceDemandingBehaviour). */
    public BranchAction branch(String n, Object... probAndBehaviour) {
        BranchAction b = set(S.createBranchAction(), "act", n);
        for (int i = 0; i < probAndBehaviour.length; i += 2) {
            ProbabilisticBranchTransition t = set(S.createProbabilisticBranchTransition(), "bt", n + "_" + i / 2);
            t.setBranchProbability((Double) probAndBehaviour[i]);
            t.setBranchBehaviour_BranchTransition((ResourceDemandingBehaviour) probAndBehaviour[i + 1]);
            b.getBranches_Branch().add(t);
        }
        return b;
    }

    /** Guarded branch: pairs of (String condition, ResourceDemandingBehaviour). */
    public BranchAction guarded(String n, Object... condAndBehaviour) {
        BranchAction b = set(S.createBranchAction(), "act", n);
        for (int i = 0; i < condAndBehaviour.length; i += 2) {
            GuardedBranchTransition t = set(S.createGuardedBranchTransition(), "bt", n + "_" + i / 2);
            t.setBranchCondition_GuardedBranchTransition(rv((String) condAndBehaviour[i]));
            t.setBranchBehaviour_BranchTransition((ResourceDemandingBehaviour) condAndBehaviour[i + 1]);
            b.getBranches_Branch().add(t);
        }
        return b;
    }

    public LoopAction loop(String n, String iterations, ResourceDemandingBehaviour body) {
        LoopAction l = set(S.createLoopAction(), "act", n);
        l.setIterationCount_LoopAction(rv(iterations));
        l.setBodyBehaviour_Loop(body);
        return l;
    }

    public CollectionIteratorAction iterate(String n, Parameter p, ResourceDemandingBehaviour body) {
        CollectionIteratorAction l = set(S.createCollectionIteratorAction(), "act", n);
        l.setParameter_CollectionIteratorAction(p);
        l.setBodyBehaviour_Loop(body);
        return l;
    }

    public ForkedBehaviour forked(AbstractAction... actions) {
        ForkedBehaviour b = S.createForkedBehaviour();
        b.setId(id("fb"));
        return chain(b, actions);
    }

    public ForkAction fork(String n, List<ForkedBehaviour> async, List<ForkedBehaviour> sync) {
        ForkAction f = set(S.createForkAction(), "act", n);
        f.getAsynchronousForkedBehaviours_ForkAction().addAll(async);
        if (sync != null && !sync.isEmpty()) {
            SynchronisationPoint sp = S.createSynchronisationPoint();
            sp.setId(id("sp"));
            sp.getSynchronousForkedBehaviours_SynchronisationPoint().addAll(sync);
            f.setSynchronisingBehaviours_ForkAction(sp);
        }
        return f;
    }

    public AcquireAction acquire(String n, PassiveResource p) {
        AcquireAction a = set(S.createAcquireAction(), "act", n);
        a.setPassiveresource_AcquireAction(p);
        return a;
    }

    public ReleaseAction release(String n, PassiveResource p) {
        ReleaseAction a = set(S.createReleaseAction(), "act", n);
        a.setPassiveResource_ReleaseAction(p);
        return a;
    }

    public SetVariableAction setVar(String n, VariableUsage... vus) {
        SetVariableAction a = set(S.createSetVariableAction(), "act", n);
        for (VariableUsage v : vus) {
            a.getLocalVariableUsages_SetVariableAction().add(v);
        }
        return a;
    }

    // ------------------------------------------------------------------ variables

    /** Variable usage: name may be dotted ("items.INNER"); pairs of (type, spec). */
    public static VariableUsage vu(String ref, Object... typeAndSpec) {
        VariableUsage v = ParameterFactory.eINSTANCE.createVariableUsage();
        v.setNamedReference__VariableUsage(ref(ref));
        for (int i = 0; i < typeAndSpec.length; i += 2) {
            VariableCharacterisation c = ParameterFactory.eINSTANCE.createVariableCharacterisation();
            c.setType((VariableCharacterisationType) typeAndSpec[i]);
            c.setSpecification_VariableCharacterisation(rv((String) typeAndSpec[i + 1]));
            v.getVariableCharacterisation_VariableUsage().add(c);
        }
        return v;
    }

    public static AbstractNamedReference ref(String dotted) {
        String[] parts = dotted.split("\\.");
        AbstractNamedReference inner = null;
        for (int i = parts.length - 1; i >= 0; i--) {
            if (inner == null) {
                VariableReference r = StoexFactory.eINSTANCE.createVariableReference();
                r.setReferenceName(parts[i]);
                inner = r;
            } else {
                NamespaceReference n = StoexFactory.eINSTANCE.createNamespaceReference();
                n.setReferenceName(parts[i]);
                n.setInnerReference_NamespaceReference(inner);
                inner = n;
            }
        }
        return inner;
    }

    public static final VariableCharacterisationType VALUE = VariableCharacterisationType.VALUE;
    public static final VariableCharacterisationType BYTESIZE = VariableCharacterisationType.BYTESIZE;
    public static final VariableCharacterisationType NUMBER_OF_ELEMENTS = VariableCharacterisationType.NUMBER_OF_ELEMENTS;
    public static final VariableCharacterisationType TYPE = VariableCharacterisationType.TYPE;
    public static final VariableCharacterisationType STRUCTURE = VariableCharacterisationType.STRUCTURE;

    // ------------------------------------------------------------------ system / composition

    public AssemblyContext assembly(ComposedStructure in, RepositoryComponent c, String n) {
        AssemblyContext a = set(C.createAssemblyContext(), "ac", n);
        a.setEncapsulatedComponent__AssemblyContext(c);
        in.getAssemblyContexts__ComposedStructure().add(a);
        return a;
    }

    public AssemblyContext assembly(RepositoryComponent c) {
        return assembly(sys, c, "Assembly_" + c.getEntityName());
    }

    public AssemblyConnector connect(ComposedStructure in, AssemblyContext from, OperationRequiredRole req,
            AssemblyContext to, OperationProvidedRole prov) {
        AssemblyConnector c = set(C.createAssemblyConnector(), "conn", "Connector");
        c.setRequiringAssemblyContext_AssemblyConnector(from);
        c.setRequiredRole_AssemblyConnector(req);
        c.setProvidingAssemblyContext_AssemblyConnector(to);
        c.setProvidedRole_AssemblyConnector(prov);
        in.getConnectors__ComposedStructure().add(c);
        return c;
    }

    public AssemblyConnector connect(AssemblyContext from, OperationRequiredRole req, AssemblyContext to,
            OperationProvidedRole prov) {
        return connect(sys, from, req, to, prov);
    }

    public AssemblyInfrastructureConnector connectInfra(ComposedStructure in, AssemblyContext from,
            InfrastructureRequiredRole req, AssemblyContext to, InfrastructureProvidedRole prov) {
        AssemblyInfrastructureConnector c = set(C.createAssemblyInfrastructureConnector(), "conn", "InfraConnector");
        c.setRequiringAssemblyContext__AssemblyInfrastructureConnector(from);
        c.setRequiredRole__AssemblyInfrastructureConnector(req);
        c.setProvidingAssemblyContext__AssemblyInfrastructureConnector(to);
        c.setProvidedRole__AssemblyInfrastructureConnector(prov);
        in.getConnectors__ComposedStructure().add(c);
        return c;
    }

    /** Outer provided role on the composed structure delegating to an inner assembly's provided role. */
    public OperationProvidedRole delegateProvided(ComposedStructure outer, OperationInterface i, AssemblyContext inner,
            OperationProvidedRole innerRole) {
        OperationProvidedRole r = provides((InterfaceProvidingRequiringEntity) outer, i);
        ProvidedDelegationConnector d = set(C.createProvidedDelegationConnector(), "conn", "ProvDelegation");
        d.setOuterProvidedRole_ProvidedDelegationConnector(r);
        d.setInnerProvidedRole_ProvidedDelegationConnector(innerRole);
        d.setAssemblyContext_ProvidedDelegationConnector(inner);
        outer.getConnectors__ComposedStructure().add(d);
        return r;
    }

    /** Outer required role on the composed structure; inner assembly's required role delegates to it. */
    public OperationRequiredRole delegateRequired(ComposedStructure outer, OperationInterface i, AssemblyContext inner,
            OperationRequiredRole innerRole) {
        OperationRequiredRole r = requires((InterfaceProvidingRequiringEntity) outer, i);
        RequiredDelegationConnector d = set(C.createRequiredDelegationConnector(), "conn", "ReqDelegation");
        d.setOuterRequiredRole_RequiredDelegationConnector(r);
        d.setInnerRequiredRole_RequiredDelegationConnector(innerRole);
        d.setAssemblyContext_RequiredDelegationConnector(inner);
        outer.getConnectors__ComposedStructure().add(d);
        return r;
    }

    /** Assembly context configuration parameter override. */
    public void configParameter(AssemblyContext a, VariableUsage vu) {
        a.getConfigParameterUsages__AssemblyContext().add(vu);
    }

    // ------------------------------------------------------------------ resource environment / allocation

    public ResourceContainer container(String n) {
        ResourceContainer c = set(RE.createResourceContainer(), "rc", n);
        env.getResourceContainer_ResourceEnvironment().add(c);
        return c;
    }

    /** A resource container nested in {@code parent} (SimuLizar 5.2.2 does not simulate it). */
    public ResourceContainer nestedContainer(ResourceContainer parent, String n) {
        ResourceContainer c = set(RE.createResourceContainer(), "rc", n);
        parent.getNestedResourceContainers__ResourceContainer().add(c);
        return c;
    }

    public ProcessingResourceSpecification resource(ResourceContainer c, ProcessingResourceType t,
            SchedulingPolicy p, String rate, int replicas) {
        ProcessingResourceSpecification s = RE.createProcessingResourceSpecification();
        s.setId(id("prs"));
        s.setActiveResourceType_ActiveResourceSpecification(t);
        s.setSchedulingPolicy(p);
        s.setProcessingRate_ProcessingResourceSpecification(rv(rate));
        s.setNumberOfReplicas(replicas);
        s.setMTTF(0.0);
        s.setMTTR(0.0);
        c.getActiveResourceSpecifications_ResourceContainer().add(s);
        return s;
    }

    public HDDProcessingResourceSpecification hdd(ResourceContainer c, SchedulingPolicy p, String rate,
            String readRate, String writeRate) {
        HDDProcessingResourceSpecification s = RE.createHDDProcessingResourceSpecification();
        s.setId(id("prs"));
        s.setActiveResourceType_ActiveResourceSpecification(HDD);
        s.setSchedulingPolicy(p);
        s.setProcessingRate_ProcessingResourceSpecification(rv(rate));
        s.setReadProcessingRate(rv(readRate));
        s.setWriteProcessingRate(rv(writeRate));
        s.setNumberOfReplicas(1);
        c.getActiveResourceSpecifications_ResourceContainer().add(s);
        return s;
    }

    public LinkingResource link(String n, String latency, String throughput, ResourceContainer... cs) {
        LinkingResource l = set(RE.createLinkingResource(), "link", n);
        CommunicationLinkResourceSpecification s = RE.createCommunicationLinkResourceSpecification();
        s.setId(id("clrs"));
        s.setCommunicationLinkResourceType_CommunicationLinkResourceSpecification(LAN);
        s.setLatency_CommunicationLinkResourceSpecification(rv(latency));
        s.setThroughput_CommunicationLinkResourceSpecification(rv(throughput));
        s.setFailureProbability(0.0);
        l.setCommunicationLinkResourceSpecifications_LinkingResource(s);
        for (ResourceContainer c : cs) {
            l.getConnectedResourceContainers_LinkingResource().add(c);
        }
        env.getLinkingResources__ResourceEnvironment().add(l);
        return l;
    }

    public AllocationContext allocate(AssemblyContext a, ResourceContainer c) {
        AllocationContext ac = set(AllocationFactory.eINSTANCE.createAllocationContext(), "alc",
                "Allocation_" + a.getEntityName());
        ac.setAssemblyContext_AllocationContext(a);
        ac.setResourceContainer_AllocationContext(c);
        alloc.getAllocationContexts_Allocation().add(ac);
        return ac;
    }

    // ------------------------------------------------------------------ usage model

    public UsageScenario scenario(String n, Workload w, AbstractUserAction... actions) {
        UsageScenario s = set(U.createUsageScenario(), "us", n);
        s.setWorkload_UsageScenario(w);
        s.setScenarioBehaviour_UsageScenario(userBehaviour(actions));
        um.getUsageScenario_UsageModel().add(s);
        return s;
    }

    public ClosedWorkload closed(int population, String think) {
        ClosedWorkload w = U.createClosedWorkload();
        w.setPopulation(population);
        w.setThinkTime_ClosedWorkload(rv(think));
        return w;
    }

    public OpenWorkload open(String interarrival) {
        OpenWorkload w = U.createOpenWorkload();
        w.setInterArrivalTime_OpenWorkload(rv(interarrival));
        return w;
    }

    public ScenarioBehaviour userBehaviour(AbstractUserAction... actions) {
        ScenarioBehaviour b = set(U.createScenarioBehaviour(), "sb", "behaviour");
        AbstractUserAction start = set(U.createStart(), "ua", "start");
        AbstractUserAction stop = set(U.createStop(), "ua", "stop");
        b.getActions_ScenarioBehaviour().add(start);
        AbstractUserAction prev = start;
        for (AbstractUserAction a : actions) {
            b.getActions_ScenarioBehaviour().add(a);
            a.setPredecessor(prev);
            prev = a;
        }
        b.getActions_ScenarioBehaviour().add(stop);
        stop.setPredecessor(prev);
        return b;
    }

    public EntryLevelSystemCall elsc(String n, OperationProvidedRole systemRole, OperationSignature s,
            VariableUsage... inputs) {
        EntryLevelSystemCall c = set(U.createEntryLevelSystemCall(), "ua", n);
        c.setProvidedRole_EntryLevelSystemCall(systemRole);
        c.setOperationSignature__EntryLevelSystemCall(s);
        for (VariableUsage v : inputs) {
            c.getInputParameterUsages_EntryLevelSystemCall().add(v);
        }
        return c;
    }

    public Delay delay(String n, String spec) {
        Delay d = set(U.createDelay(), "ua", n);
        d.setTimeSpecification_Delay(rv(spec));
        return d;
    }

    public Loop uloop(String n, String iterations, AbstractUserAction... body) {
        Loop l = set(U.createLoop(), "ua", n);
        l.setLoopIteration_Loop(rv(iterations));
        l.setBodyBehaviour_Loop(userBehaviour(body));
        return l;
    }

    /** Usage branch: pairs of (Double probability, ScenarioBehaviour). */
    public Branch ubranch(String n, Object... probAndBehaviour) {
        Branch b = set(U.createBranch(), "ua", n);
        for (int i = 0; i < probAndBehaviour.length; i += 2) {
            BranchTransition t = U.createBranchTransition();
            t.setBranchProbability((Double) probAndBehaviour[i]);
            t.setBranchedBehaviour_BranchTransition((ScenarioBehaviour) probAndBehaviour[i + 1]);
            b.getBranchTransitions_Branch().add(t);
        }
        return b;
    }

    // ------------------------------------------------------------------ saving

    /** Saves all models (+ default monitors) into dir as &lt;name&gt;.&lt;ext&gt;. Returns written files. */
    public List<File> save(File dir) throws Exception {
        dir.mkdirs();
        List<File> out = new ArrayList<>();
        Object[][] models = { { repo, "repository" }, { sys, "system" }, { env, "resourceenvironment" },
                { alloc, "allocation" }, { um, "usagemodel" } };
        List<Resource> res = new ArrayList<>();
        for (Object[] m : models) {
            File f = new File(dir, name + "." + m[1]);
            Resource r = rs.createResource(URI.createFileURI(f.getAbsolutePath()));
            r.getContents().add((EObject) m[0]);
            res.add(r);
            out.add(f);
        }
        Monitors.addDefaultMonitors(rs, um, alloc, dir, name);
        for (Resource r : rs.getResources()) {
            if (r.getURI().isFile()) {
                r.save(Monitors.SAVE_OPTIONS);
            }
        }
        return out;
    }

    public static String entityName(EObject o) {
        return o instanceof Entity ? ((Entity) o).getEntityName() : o.eClass().getName();
    }
}
