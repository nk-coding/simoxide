#!/usr/bin/env python3
"""Generates the Slingshot semantics probe models (reference/slingshot/models/probes/<name>/).

Each probe is a minimal PCM model that demonstrates one or a few rules of the Slingshot
specification (docs/spec/slingshot/, rules SS-*). The probes are described compactly in PROBES
below and written as XMI files that both Slingshot and SimOxide load: repository, system,
resource environment, allocation, usage model, measuring points, monitor repository, plus
probe.json (simulated time, seed, rules, description).

    tools/probegen.py [OUTDIR] [NAME...]        default OUTDIR: models/probes

The ids are "_<probe>_<local>", so traces and measurements stay readable. Expected outputs are
produced by verify-det.sh / probes.sh, not here.
"""
import json
import os
import sys
from xml.sax.saxutils import quoteattr

H = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

CPU = "_oro4gG3fEdy4YaaT-RYrLQ"
HDD = "_BIjHoQ3KEdyouMqirZIhzQ"
DELAY = "_nvHX4KkREdyEA_b89s7q9w"
LAN = "_o3sScH2AEdyH8uerKnHYug"
RT = "pathmap://PCM_MODELS/Palladio.resourcetype#"
METRIC = {
    "rt": ("NumericalBaseMetricDescription", "_6rYmYs7nEeOX_4BzImuHbA"),  # Response Time
    "demand": ("NumericalBaseMetricDescription", "_eg_F0s7qEeOX_4BzImuHbA"),  # Resource Demand
    "state": ("NumericalBaseMetricDescription", "_paDhIs7qEeOX_4BzImuHbA"),  # State of Active Resource
    "util": ("NumericalBaseMetricDescription", "_QIb6cikUEeSuf8LV7cHLgA"),  # Utilization of Active Resource
    "wait": ("NumericalBaseMetricDescription", "_QWjAYs7qEeOX_4BzImuHbA"),  # Waiting Time
    "pstate": ("NumericalBaseMetricDescription", "_x0-pks7rEeOX_4BzImuHbA"),  # State of Passive Resource
    "hold": ("NumericalBaseMetricDescription", "_zETOUs7pEeOX_4BzImuHbA"),  # Holding Time
}
NS = {
    "xmi": "http://www.omg.org/XMI",
    "xsi": "http://www.w3.org/2001/XMLSchema-instance",
    "repository": "http://palladiosimulator.org/PalladioComponentModel/Repository/5.2",
    "seff": "http://palladiosimulator.org/PalladioComponentModel/SEFF/5.2",
    "stoex": "http://sdq.ipd.uka.de/StochasticExpressions/2.2",
    "composition": "http://palladiosimulator.org/PalladioComponentModel/Core/Composition/5.2",
    "system": "http://palladiosimulator.org/PalladioComponentModel/System/5.2",
    "resourceenvironment": "http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2",
    "allocation": "http://palladiosimulator.org/PalladioComponentModel/Allocation/5.2",
    "usagemodel": "http://palladiosimulator.org/PalladioComponentModel/UsageModel/5.2",
    "mp": "http://palladiosimulator.org/EDP2/MeasuringPoint/1.0",
    "pcmmeasuringpoint": "http://palladiosimulator.org/PCM/MeasuringPoint/1.0",
    "monitorrepository": "http://palladiosimulator.org/MonitorRepository/1.0",
    "metricspec": "http://palladiosimulator.org/MetricSpec/1.0",
}


def a(**kw):
    """XML attributes in the given order; None values are left out."""
    return "".join(f" {k}={quoteattr(str(v))}" for k, v in kw.items() if v is not None)


def ns(*names):
    return "".join(f' xmlns:{n}="{NS[n]}"' for n in names)


class Model:
    """Builder of one probe model. Element ids are _<probe>_<local>."""

    def __init__(self, name):
        self.name = name
        self.n = {}
        self.containers = []  # (local, name, [spec dict])
        self.links = []  # (local, name, [container locals], latency, throughput, failure)
        self.interfaces = []  # (local, name, [sig locals])
        self.components = []  # dict
        self.assemblies = []  # (local, name, component local)
        self.delegations = []  # (local, system role local, assembly local, inner role local, interface local)
        self.connectors = []  # (local, requiring ac, required role, providing ac, provided role)
        self.allocs = []  # (local, ac local, container local)
        self.scenarios = []  # dict
        self.monitors = []  # (mp xml, [metrics])

    def id(self, local):
        return f"_{self.name}_{local}"

    def fresh(self, kind):
        self.n[kind] = self.n.get(kind, 0) + 1
        return f"{kind}{self.n[kind]}"

    # --- resource environment -----------------------------------------------------------------
    def container(self, local, name, *specs):
        """specs: dicts {id, type: CPU|HDD|DELAY, policy: ProcessorSharing|FCFS|Delay, rate, replicas}"""
        self.containers.append((local, name, list(specs)))
        return local

    def link(self, local, name, containers, latency="0.0", throughput="1.0", failure=None):
        self.links.append((local, name, containers, latency, throughput, failure))

    # --- repository ----------------------------------------------------------------------------
    def interface(self, local, name, *sigs):
        self.interfaces.append((local, name, list(sigs)))

    def component(self, local, name, provides=(), requires=(), passive=(), seffs=(), params=()):
        """provides: [(role local, interface local)], requires: [(role local, interface local)],
        passive: [(local, name, capacity)], seffs: [(sig local, steps)], params: component parameter
        usages [(var, char, spec)]"""
        self.components.append(dict(local=local, name=name, provides=list(provides), requires=list(requires),
                                    passive=list(passive), seffs=list(seffs), params=list(params)))

    # --- system / allocation ---------------------------------------------------------------------
    def assembly(self, local, name, comp, params=()):
        self.assemblies.append((local, name, comp, list(params)))

    def delegate(self, local, outer_role, ac, inner_role, interface):
        self.delegations.append((local, outer_role, ac, inner_role, interface))

    def connect(self, local, requiring_ac, required_role, providing_ac, provided_role):
        self.connectors.append((local, requiring_ac, required_role, providing_ac, provided_role))

    def allocate(self, local, ac, rc):
        self.allocs.append((local, ac, rc))

    # --- usage -----------------------------------------------------------------------------------
    def scenario(self, local, name, workload, steps):
        """workload: ("open", interarrival) or ("closed", population, think)"""
        self.scenarios.append(dict(local=local, name=name, workload=workload, steps=steps))

    # --- monitoring ------------------------------------------------------------------------------
    def monitor(self, mp, *metrics):
        """mp: ("scenario", us) | ("resource", spec local, replica) | ("assemblyop", ac, role, sig)
        | ("elsc", ua) | ("sysop", role, sig) | ("passive", ac, passive local)"""
        self.monitors.append((mp, list(metrics)))

    # ============================================================================================
    def write(self, out):
        os.makedirs(out, exist_ok=True)
        files = {
            "repository": self.repository(), "system": self.system(), "resourceenvironment": self.resenv(),
            "allocation": self.allocation(), "usagemodel": self.usagemodel(),
            "measuringpoint": self.measuringpoints(), "monitorrepository": self.monitorrepository(),
        }
        for ext, text in files.items():
            with open(os.path.join(out, f"{self.name}.{ext}"), "w") as f:
                f.write(text)

    def f(self, ext):
        return f"{self.name}.{ext}"

    # --- XMI writers ---------------------------------------------------------------------------
    def resenv(self):
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<resourceenvironment:ResourceEnvironment xmi:version="2.0"{ns("xmi", "resourceenvironment")}{a(entityName=self.name)}>']
        for (local, name, cs, lat, thr, fail) in self.links:
            o.append(f'  <linkingResources__ResourceEnvironment{a(id=self.id(local), entityName=name, connectedResourceContainers_LinkingResource=" ".join(self.id(c) for c in cs))}>')
            o.append(f'    <communicationLinkResourceSpecifications_LinkingResource{a(id=self.id(local + "_spec"), failureProbability=fail)}>')
            o.append(f'      <communicationLinkResourceType_CommunicationLinkResourceSpecification href="{RT}{LAN}"/>')
            o.append(f'      <latency_CommunicationLinkResourceSpecification{a(specification=lat)}/>')
            o.append(f'      <throughput_CommunicationLinkResourceSpecification{a(specification=thr)}/>')
            o.append('    </communicationLinkResourceSpecifications_LinkingResource>')
            o.append('  </linkingResources__ResourceEnvironment>')
        for (local, name, specs) in self.containers:
            o.append(f'  <resourceContainer_ResourceEnvironment{a(id=self.id(local), entityName=name)}>')
            for s in specs:
                typ = {"CPU": CPU, "HDD": HDD, "DELAY": DELAY}[s.get("type", "CPU")]
                o.append(f'    <activeResourceSpecifications_ResourceContainer{a(id=self.id(s["id"]), numberOfReplicas=s.get("replicas"))}>')
                o.append(f'      <schedulingPolicy href="{RT}{s.get("policy", "ProcessorSharing")}"/>')
                o.append(f'      <activeResourceType_ActiveResourceSpecification href="{RT}{typ}"/>')
                o.append(f'      <processingRate_ProcessingResourceSpecification{a(specification=s.get("rate", "1.0"))}/>')
                o.append('    </activeResourceSpecifications_ResourceContainer>')
            o.append('  </resourceContainer_ResourceEnvironment>')
        o.append('</resourceenvironment:ResourceEnvironment>')
        return "\n".join(o) + "\n"

    def var_usages(self, tag, usages, ind):
        o = []
        for (var, char, spec) in usages:
            o.append(f'{ind}<{tag}>')
            o.append(f'{ind}  <variableCharacterisation_VariableUsage{a(type=char)}>')
            o.append(f'{ind}    <specification_VariableCharacterisation{a(specification=spec)}/>')
            o.append(f'{ind}  </variableCharacterisation_VariableUsage>')
            if "." in var:
                outer, inner = var.split(".", 1)
                o.append(f'{ind}  <namedReference__VariableUsage xsi:type="stoex:NamespaceReference"{a(referenceName=outer)}>')
                o.append(f'{ind}    <innerReference_NamespaceReference xsi:type="stoex:VariableReference"{a(referenceName=inner)}/>')
                o.append(f'{ind}  </namedReference__VariableUsage>')
            else:
                o.append(f'{ind}  <namedReference__VariableUsage xsi:type="stoex:VariableReference"{a(referenceName=var)}/>')
            o.append(f'{ind}</{tag}>')
        return o

    def seff_steps(self, steps, ind):
        """steps: list of tuples; Start/Stop are added. Returns XMI lines."""
        ids = [self.fresh("act") for _ in range(len(steps) + 2)]
        o = []
        o.append(f'{ind}<steps_Behaviour xsi:type="seff:StartAction"{a(id=self.id(ids[0]), entityName="start", successor_AbstractAction=self.id(ids[1]))}/>')
        for i, st in enumerate(steps, start=1):
            kind = st[0]
            name = st[1]
            pre, suc = self.id(ids[i - 1]), self.id(ids[i + 1])
            common = dict(id=self.id(ids[i]), entityName=name, predecessor_AbstractAction=pre, successor_AbstractAction=suc)
            if kind == "internal":  # ("internal", name, [(type, spec)], [(infra role, infra sig, calls)])
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:InternalAction"{a(**common)}>')
                for (typ, spec) in st[2]:
                    o.append(f'{ind}  <resourceDemand_Action>')
                    o.append(f'{ind}    <specification_ParametericResourceDemand{a(specification=spec)}/>')
                    o.append(f'{ind}    <requiredResource_ParametricResourceDemand href="{RT}{ {"CPU": CPU, "HDD": HDD, "DELAY": DELAY}[typ]}"/>')
                    o.append(f'{ind}  </resourceDemand_Action>')
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "external":  # ("external", name, role, sig, inputs, returns)
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:ExternalCallAction"{a(**common, calledService_ExternalService=self.id(st[3]), role_ExternalService=self.id(st[2]))}>')
                o += self.var_usages("inputVariableUsages__CallAction", st[4] if len(st) > 4 else [], ind + "  ")
                o += self.var_usages("returnVariableUsage__CallReturnAction", st[5] if len(st) > 5 else [], ind + "  ")
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "branch":  # ("branch", name, [(prob, steps)])
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:BranchAction"{a(**common)}>')
                for (p, sub) in st[2]:
                    bt = self.fresh("bt")
                    o.append(f'{ind}  <branches_Branch xsi:type="seff:ProbabilisticBranchTransition"{a(id=self.id(bt), entityName=bt, branchProbability=p)}>')
                    o.append(f'{ind}    <branchBehaviour_BranchTransition{a(id=self.id(self.fresh("rdb")))}>')
                    o += self.seff_steps(sub, ind + "      ")
                    o.append(f'{ind}    </branchBehaviour_BranchTransition>')
                    o.append(f'{ind}  </branches_Branch>')
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "guard":  # ("guard", name, [(condition, steps)])
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:BranchAction"{a(**common)}>')
                for (cond, sub) in st[2]:
                    bt = self.fresh("bt")
                    o.append(f'{ind}  <branches_Branch xsi:type="seff:GuardedBranchTransition"{a(id=self.id(bt), entityName=bt)}>')
                    o.append(f'{ind}    <branchBehaviour_BranchTransition{a(id=self.id(self.fresh("rdb")))}>')
                    o += self.seff_steps(sub, ind + "      ")
                    o.append(f'{ind}    </branchBehaviour_BranchTransition>')
                    o.append(f'{ind}    <branchCondition_GuardedBranchTransition{a(specification=cond)}/>')
                    o.append(f'{ind}  </branches_Branch>')
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "loop":  # ("loop", name, count, steps)
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:LoopAction"{a(**common)}>')
                o.append(f'{ind}  <bodyBehaviour_Loop{a(id=self.id(self.fresh("rdb")))}>')
                o += self.seff_steps(st[3], ind + "    ")
                o.append(f'{ind}  </bodyBehaviour_Loop>')
                o.append(f'{ind}  <iterationCount_LoopAction{a(specification=st[2])}/>')
                o.append(f'{ind}</steps_Behaviour>')
            elif kind in ("acquire", "release"):  # ("acquire", name, passive local, amount or None)
                attr = "passiveresource_AcquireAction" if kind == "acquire" else "passiveResource_ReleaseAction"
                typ = "AcquireAction" if kind == "acquire" else "ReleaseAction"
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:{typ}"{a(**common)} {attr}="{self.id(st[2])}">')
                if st[3] is not None:
                    o.append(f'{ind}  <resourceDemand_Action>')
                    o.append(f'{ind}    <specification_ParametericResourceDemand{a(specification=st[3])}/>')
                    o.append(f'{ind}    <requiredResource_ParametricResourceDemand href="{RT}{CPU}"/>')
                    o.append(f'{ind}  </resourceDemand_Action>')
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "setvar":  # ("setvar", name, [(var, char, spec)])
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:SetVariableAction"{a(**common)}>')
                o += self.var_usages("localVariableUsages_SetVariableAction", st[2], ind + "  ")
                o.append(f'{ind}</steps_Behaviour>')
            elif kind == "fork":  # ("fork", name, [sync steps...], [async steps...])
                o.append(f'{ind}<steps_Behaviour xsi:type="seff:ForkAction"{a(**common)}>')
                for sub in st[3]:
                    o.append(f'{ind}  <asynchronousForkedBehaviours_ForkAction{a(id=self.id(self.fresh("fb")))}>')
                    o += self.seff_steps(sub, ind + "    ")
                    o.append(f'{ind}  </asynchronousForkedBehaviours_ForkAction>')
                if st[2]:
                    o.append(f'{ind}  <synchronisingBehaviours_ForkAction{a(id=self.id(self.fresh("sp")))}>')
                    for sub in st[2]:
                        o.append(f'{ind}    <synchronousForkedBehaviours_SynchronisationPoint{a(id=self.id(self.fresh("fb")))}>')
                        o += self.seff_steps(sub, ind + "      ")
                        o.append(f'{ind}    </synchronousForkedBehaviours_SynchronisationPoint>')
                    o.append(f'{ind}  </synchronisingBehaviours_ForkAction>')
                o.append(f'{ind}</steps_Behaviour>')
            else:
                raise ValueError(kind)
        o.append(f'{ind}<steps_Behaviour xsi:type="seff:StopAction"{a(id=self.id(ids[-1]), entityName="stop", predecessor_AbstractAction=self.id(ids[-2]))}/>')
        return o

    def repository(self):
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<repository:Repository xmi:version="2.0"{ns("xmi", "xsi", "repository", "seff", "stoex")}{a(id=self.id("repository"), entityName=self.name)}>']
        for c in self.components:
            o.append(f'  <components__Repository xsi:type="repository:BasicComponent"{a(id=self.id(c["local"]), entityName=c["name"])}>')
            for (role, itf) in c["provides"]:
                o.append(f'    <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole"{a(id=self.id(role), entityName=role, providedInterface__OperationProvidedRole=self.id(itf))}/>')
            for (role, itf) in c["requires"]:
                o.append(f'    <requiredRoles_InterfaceRequiringEntity xsi:type="repository:OperationRequiredRole"{a(id=self.id(role), entityName=role, requiredInterface__OperationRequiredRole=self.id(itf))}/>')
            for (sig, steps) in c["seffs"]:
                o.append(f'    <serviceEffectSpecifications__BasicComponent xsi:type="seff:ResourceDemandingSEFF"{a(id=self.id(self.fresh("seff")), describedService__SEFF=self.id(sig))}>')
                o += self.seff_steps(steps, "      ")
                o.append('    </serviceEffectSpecifications__BasicComponent>')
            for (local, name, cap) in c["passive"]:
                o.append(f'    <passiveResource_BasicComponent{a(id=self.id(local), entityName=name)}>')
                o.append(f'      <capacity_PassiveResource{a(specification=cap)}/>')
                o.append('    </passiveResource_BasicComponent>')
            o += self.var_usages("componentParameterUsage_ImplementationComponentType", c["params"], "    ")
            o.append('  </components__Repository>')
        for (local, name, sigs) in self.interfaces:
            o.append(f'  <interfaces__Repository xsi:type="repository:OperationInterface"{a(id=self.id(local), entityName=name)}>')
            for s in sigs:
                o.append(f'    <signatures__OperationInterface{a(id=self.id(s), entityName=s)}/>')
            o.append('  </interfaces__Repository>')
        o.append('</repository:Repository>')
        return "\n".join(o) + "\n"

    def system(self):
        r = self.f("repository")
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<system:System xmi:version="2.0"{ns("xmi", "xsi", "composition", "repository", "system", "stoex")}{a(id=self.id("system"), entityName=self.name)}>']
        for (local, name, comp, params) in self.assemblies:
            o.append(f'  <assemblyContexts__ComposedStructure{a(id=self.id(local), entityName=name)}>')
            o.append(f'    <encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="{r}#{self.id(comp)}"/>')
            o += self.var_usages("configParameterUsages__AssemblyContext", params, "    ")
            o.append('  </assemblyContexts__ComposedStructure>')
        for (local, req_ac, req_role, prov_ac, prov_role) in self.connectors:
            o.append(f'  <connectors__ComposedStructure xsi:type="composition:AssemblyConnector"{a(id=self.id(local), entityName=local, requiringAssemblyContext_AssemblyConnector=self.id(req_ac), providingAssemblyContext_AssemblyConnector=self.id(prov_ac))}>')
            o.append(f'    <providedRole_AssemblyConnector href="{r}#{self.id(prov_role)}"/>')
            o.append(f'    <requiredRole_AssemblyConnector href="{r}#{self.id(req_role)}"/>')
            o.append('  </connectors__ComposedStructure>')
        for (local, outer, ac, inner, itf) in self.delegations:
            o.append(f'  <connectors__ComposedStructure xsi:type="composition:ProvidedDelegationConnector"{a(id=self.id(local), entityName=local, outerProvidedRole_ProvidedDelegationConnector=self.id(outer), assemblyContext_ProvidedDelegationConnector=self.id(ac))}>')
            o.append(f'    <innerProvidedRole_ProvidedDelegationConnector href="{r}#{self.id(inner)}"/>')
            o.append('  </connectors__ComposedStructure>')
        seen = set()
        for (local, outer, ac, inner, itf) in self.delegations:
            if outer in seen:
                continue
            seen.add(outer)
            o.append(f'  <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole"{a(id=self.id(outer), entityName=outer)}>')
            o.append(f'    <providedInterface__OperationProvidedRole href="{r}#{self.id(itf)}"/>')
            o.append('  </providedRoles_InterfaceProvidingEntity>')
        o.append('</system:System>')
        return "\n".join(o) + "\n"

    def allocation(self):
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<allocation:Allocation xmi:version="2.0"{ns("xmi", "allocation")}{a(id=self.id("allocation"), entityName=self.name)}>',
             f'  <targetResourceEnvironment_Allocation href="{self.f("resourceenvironment")}#/"/>',
             f'  <system_Allocation href="{self.f("system")}#{self.id("system")}"/>']
        for (local, ac, rc) in self.allocs:
            o.append(f'  <allocationContexts_Allocation{a(id=self.id(local), entityName=local)}>')
            o.append(f'    <resourceContainer_AllocationContext href="{self.f("resourceenvironment")}#{self.id(rc)}"/>')
            o.append(f'    <assemblyContext_AllocationContext href="{self.f("system")}#{self.id(ac)}"/>')
            o.append('  </allocationContexts_Allocation>')
        o.append('</allocation:Allocation>')
        return "\n".join(o) + "\n"

    def usage_steps(self, steps, ind, name):
        ids = [self.fresh("ua") for _ in range(len(steps) + 2)]
        o = [f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:Start"{a(id=self.id(ids[0]), entityName="start", successor=self.id(ids[1]))}/>']
        for i, st in enumerate(steps, start=1):
            common = dict(id=self.id(ids[i]), entityName=st[1], successor=self.id(ids[i + 1]), predecessor=self.id(ids[i - 1]))
            kind = st[0]
            if kind == "call":  # ("call", name, system role, sig, inputs, outputs)
                o.append(f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:EntryLevelSystemCall"{a(**common)}>')
                o.append(f'{ind}  <providedRole_EntryLevelSystemCall href="{self.f("system")}#{self.id(st[2])}"/>')
                o.append(f'{ind}  <operationSignature__EntryLevelSystemCall href="{self.f("repository")}#{self.id(st[3])}"/>')
                o += self.var_usages("inputParameterUsages_EntryLevelSystemCall", st[4] if len(st) > 4 else [], ind + "  ")
                o += self.var_usages("outputParameterUsages_EntryLevelSystemCall", st[5] if len(st) > 5 else [], ind + "  ")
                o.append(f'{ind}</actions_ScenarioBehaviour>')
            elif kind == "delay":  # ("delay", name, spec)
                o.append(f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:Delay"{a(**common)}>')
                o.append(f'{ind}  <timeSpecification_Delay{a(specification=st[2])}/>')
                o.append(f'{ind}</actions_ScenarioBehaviour>')
            elif kind == "loop":  # ("loop", name, count, steps)
                o.append(f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:Loop"{a(**common)}>')
                o.append(f'{ind}  <loopIteration_Loop{a(specification=st[2])}/>')
                o.append(f'{ind}  <bodyBehaviour_Loop{a(id=self.id(self.fresh("sb")), entityName="body")}>')
                o += self.usage_steps(st[3], ind + "    ", "body")
                o.append(f'{ind}  </bodyBehaviour_Loop>')
                o.append(f'{ind}</actions_ScenarioBehaviour>')
            elif kind == "branch":  # ("branch", name, [(p, steps)])
                o.append(f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:Branch"{a(**common)}>')
                for (p, sub) in st[2]:
                    o.append(f'{ind}  <branchTransitions_Branch{a(branchProbability=p)}>')
                    o.append(f'{ind}    <branchedBehaviour_BranchTransition{a(id=self.id(self.fresh("sb")), entityName="branch")}>')
                    o += self.usage_steps(sub, ind + "      ", "branch")
                    o.append(f'{ind}    </branchedBehaviour_BranchTransition>')
                    o.append(f'{ind}  </branchTransitions_Branch>')
                o.append(f'{ind}</actions_ScenarioBehaviour>')
            else:
                raise ValueError(kind)
        o.append(f'{ind}<actions_ScenarioBehaviour xsi:type="usagemodel:Stop"{a(id=self.id(ids[-1]), entityName="stop", predecessor=self.id(ids[-2]))}/>')
        return o

    def usagemodel(self):
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<usagemodel:UsageModel xmi:version="2.0"{ns("xmi", "xsi", "usagemodel", "stoex")}>']
        for s in self.scenarios:
            o.append(f'  <usageScenario_UsageModel{a(id=self.id(s["local"]), entityName=s["name"])}>')
            o.append(f'    <scenarioBehaviour_UsageScenario{a(id=self.id(s["local"] + "_sb"), entityName="behaviour")}>')
            o += self.usage_steps(s["steps"], "      ", "behaviour")
            o.append('    </scenarioBehaviour_UsageScenario>')
            w = s["workload"]
            if w[0] == "open":
                o.append('    <workload_UsageScenario xsi:type="usagemodel:OpenWorkload">')
                o.append(f'      <interArrivalTime_OpenWorkload{a(specification=w[1])}/>')
            else:
                o.append(f'    <workload_UsageScenario xsi:type="usagemodel:ClosedWorkload"{a(population=w[1])}>')
                o.append(f'      <thinkTime_ClosedWorkload{a(specification=w[2])}/>')
            o.append('    </workload_UsageScenario>')
            o.append('  </usageScenario_UsageModel>')
        o.append('</usagemodel:UsageModel>')
        return "\n".join(o) + "\n"

    def measuringpoints(self):
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<mp:MeasuringPointRepository xmi:version="2.0"{ns("xmi", "xsi", "mp", "pcmmeasuringpoint", "repository")}{a(id=self.id("mpr"))}>']
        for (mp, _) in self.monitors:
            k = mp[0]
            if k == "scenario":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:UsageScenarioMeasuringPoint"{a(stringRepresentation="Usage Scenario: " + mp[1])}>')
                o.append(f'    <usageScenario href="{self.f("usagemodel")}#{self.id(mp[1])}"/>')
            elif k == "resource":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:ActiveResourceMeasuringPoint"{a(stringRepresentation=f"{mp[1]} [{mp[2]}]", replicaID=mp[2])}>')
                o.append(f'    <activeResource href="{self.f("resourceenvironment")}#{self.id(mp[1])}"/>')
            elif k == "assemblyop":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:AssemblyOperationMeasuringPoint"{a(stringRepresentation=f"{mp[1]}.{mp[2]}.{mp[3]}")}>')
                outer = {d[1] for d in self.delegations}
                rf = self.f("system") if mp[2] in outer else self.f("repository")
                o.append(f'    <role xsi:type="repository:OperationProvidedRole" href="{rf}#{self.id(mp[2])}"/>')
                o.append(f'    <operationSignature href="{self.f("repository")}#{self.id(mp[3])}"/>')
                o.append(f'    <assembly href="{self.f("system")}#{self.id(mp[1])}"/>')
            elif k == "elsc":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:EntryLevelSystemCallMeasuringPoint"{a(stringRepresentation="SystemCall " + mp[1])}>')
                o.append(f'    <entryLevelSystemCall href="{self.f("usagemodel")}#{self.id(mp[1])}"/>')
            elif k == "sysop":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:SystemOperationMeasuringPoint"{a(stringRepresentation=f"{mp[1]}.{mp[2]}")}>')
                o.append(f'    <role xsi:type="repository:OperationProvidedRole" href="{self.f("system")}#{self.id(mp[1])}"/>')
                o.append(f'    <operationSignature href="{self.f("repository")}#{self.id(mp[2])}"/>')
                o.append(f'    <system href="{self.f("system")}#{self.id("system")}"/>')
            elif k == "passive":
                o.append(f'  <measuringPoints xsi:type="pcmmeasuringpoint:AssemblyPassiveResourceMeasuringPoint"{a(stringRepresentation=f"Passive Resource: {mp[1]}.{mp[2]}")}>')
                o.append(f'    <assembly href="{self.f("system")}#{self.id(mp[1])}"/>')
                o.append(f'    <passiveResource href="{self.f("repository")}#{self.id(mp[2])}"/>')
            else:
                raise ValueError(k)
            o.append('  </measuringPoints>')
        o.append('</mp:MeasuringPointRepository>')
        return "\n".join(o) + "\n"

    def monitorrepository(self):
        types = {"scenario": "UsageScenarioMeasuringPoint", "resource": "ActiveResourceMeasuringPoint",
                 "assemblyop": "AssemblyOperationMeasuringPoint", "elsc": "EntryLevelSystemCallMeasuringPoint",
                 "sysop": "SystemOperationMeasuringPoint", "passive": "AssemblyPassiveResourceMeasuringPoint"}
        o = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<monitorrepository:MonitorRepository{ns("xsi", "metricspec", "monitorrepository", "pcmmeasuringpoint")}{a(id=self.id("monrepo"), entityName="probe monitors")}>']
        for i, (mp, metrics) in enumerate(self.monitors):
            o.append(f'  <monitors{a(id=self.id(f"mon{i + 1}"), entityName=f"{mp[0]} {mp[1]}")}>')
            for m in metrics:
                typ, mid = METRIC[m]
                o.append(f'    <measurementSpecifications{a(id=self.id(f"ms{i + 1}_{m}"), triggersSelfAdaptations="false")}>')
                o.append(f'      <metricDescription xsi:type="metricspec:{typ}" href="pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec#{mid}"/>')
                o.append(f'      <processingType xsi:type="monitorrepository:FeedThrough"{a(id=self.id(f"ft{i + 1}_{m}"))}/>')
                o.append('    </measurementSpecifications>')
            o.append(f'    <measuringPoint xsi:type="pcmmeasuringpoint:{types[mp[0]]}" href="{self.f("measuringpoint")}#//@measuringPoints.{i}"/>')
            o.append('  </monitors>')
        o.append('</monitorrepository:MonitorRepository>')
        return "\n".join(o) + "\n"


# ================================================================================================
# helpers for the common shape: one server component, one assembly, one container

def single(name, steps, workload, cpu=None, extra_specs=(), monitors=("scenario", "cpu"), sigs=("op",), seffs=None,
           passive=()):
    m = Model(name)
    cpu = cpu or dict(id="cpu", type="CPU", policy="ProcessorSharing", rate="1.0")
    m.container("rc1", "Node", cpu, *extra_specs)
    m.interface("if1", "IService", *sigs)
    m.component("comp1", "Server", provides=[("prov1", "if1")], passive=passive,
                seffs=seffs or [(sigs[0], steps)])
    m.assembly("ac1", "Assembly_Server", "comp1")
    m.delegate("deleg1", "sysprov1", "ac1", "prov1", "if1")
    m.allocate("alc1", "ac1", "rc1")
    m.scenario("us1", "Scenario", workload, [("call", "callOp", "sysprov1", sigs[0])])
    for mon in monitors:
        if mon == "scenario":
            m.monitor(("scenario", "us1"), "rt")
        elif mon == "cpu":
            m.monitor(("resource", cpu["id"], 0), "state", "demand", "util")
    return m


def p_fifo_ties():
    """3 closed users, no think time, constant demand on a 1-core PS: every
    arrival and completion is a tie. Results are FIFO in handler order; PS ties go to the earliest job
    (det patch S0-P2); stale ProcessorSharingJobProgressed events still sample state and utilisation."""
    return single("ss_fifo_ties", [("internal", "work", [("CPU", "1.0")])], ("closed", 3, "0.0")), \
        dict(sim_time=7, rules=["SS-ENG-3", "SS-BUS-6", "SS-RES-2.4", "SS-RES-2.6", "SS-MEAS-3.2"])


def p_two_demands():
    """one InternalAction with a CPU and an HDD demand, then two more actions. Both demands run
    concurrently; each completion advances the SEFF cursor, so the next actions run while the HDD demand
    is still served and the request ends before it; the HDD completion after the Stop throws, which
    disposes SystemSimulationBehavior.onActiveResourceFinished for the rest of the run."""
    m = single("ss_two_demands",
               [("internal", "both", [("CPU", "1.0"), ("HDD", "2.0")]),
                ("internal", "second", [("CPU", "0.5")]),
                ("internal", "third", [("CPU", "0.25")])],
               ("closed", 1, "1.0"),
               extra_specs=[dict(id="hdd", type="HDD", policy="FCFS", rate="1.0")])
    m.monitor(("resource", "hdd", 0), "state", "demand")
    return m, dict(sim_time=12, rules=["SS-ACT-3.2", "SS-Q17", "SS-BUS-7"])


def p_loop_zero():
    """a SEFF LoopAction whose count is 0 runs the body once and then throws
    (NoSuchElementException); the run loses SEFF progress. Iteration counts 1 and 2 behave."""
    return single("ss_loop_zero",
                  [("loop", "loop", "IntPMF[(0;0.5)(2;0.5)]", [("internal", "body", [("CPU", "0.1")])])],
                  ("open", "1.0")), dict(sim_time=8, rules=["SS-ACT-6.2", "SS-Q19", "SS-BUS-7"])


def p_usage_loop_zero():
    """a usage-model Loop with count 0 throws in its LoopScenarioBehaviorContext builder
    (precondition initial < maximal), which disposes the subscriber that interpreted it."""
    m = Model("ss_usage_loop_zero")
    m.container("rc1", "Node", dict(id="cpu", type="CPU", policy="ProcessorSharing", rate="1.0"))
    m.interface("if1", "IService", "op")
    m.component("comp1", "Server", provides=[("prov1", "if1")],
                seffs=[("op", [("internal", "work", [("CPU", "0.1")])])])
    m.assembly("ac1", "Assembly_Server", "comp1")
    m.delegate("deleg1", "sysprov1", "ac1", "prov1", "if1")
    m.allocate("alc1", "ac1", "rc1")
    m.scenario("us1", "Scenario", ("open", "1.0"),
               [("loop", "loop", "IntPMF[(0;0.5)(2;0.5)]", [("call", "callOp", "sysprov1", "op")])])
    m.monitor(("scenario", "us1"), "rt")
    return m, dict(sim_time=8, rules=["SS-WL-4.3", "SS-Q19", "SS-BUS-7"])


def p_stop_time():
    """open workload with inter-arrival 2.5 and demand 1.0, simTime 10: the arrival at exactly
    t = 10 and the completion at t = 10 never run, SimulationFinished comes first; queues are cleared
    without a drain; no end-of-run state sample."""
    m = single("ss_stop_time", [("internal", "work", [("CPU", "1.0")])], ("open", "2.5"))
    return m, dict(sim_time=10, rules=["SS-RUN-3", "SS-RUN-4"])


def p_closed_think():
    """closed workload, 2 users, think time Exp(1.0), demand 0.5 on FCFS. All users start at
    t = 0 without think time; the think time is drawn when the scenario finishes (in the UserFinished
    handler, after the response time was measured)."""
    m = single("ss_closed_think", [("internal", "work", [("CPU", "0.5")])], ("closed", 2, "Exp(1.0)"),
               cpu=dict(id="cpu", type="CPU", policy="FCFS", rate="1.0"))
    return m, dict(sim_time=6, rules=["SS-WL-3.1", "SS-RNG-2"])


def p_open_draws():
    """open workload with Exp inter-arrival and Exp demand: the inter-arrival time of the next
    user is drawn while the current user's Start is interpreted, before the current user's demand."""
    m = single("ss_open_draws", [("internal", "work", [("CPU", "Exp(2.0)")])], ("open", "Exp(1.0)"))
    return m, dict(sim_time=5, rules=["SS-WL-2.1", "SS-RNG-2"])


def p_fcfs_replicas():
    """FCFS with numberOfReplicas = 2 is one server; the service time is the whole demand,
    scheduled when the job reaches the head (no remaining-demand update, no lost time)."""
    m = single("ss_fcfs_replicas", [("internal", "work", [("CPU", "1.0")])], ("closed", 3, "0.0"),
               cpu=dict(id="cpu", type="CPU", policy="FCFS", rate="1.0", replicas=2))
    return m, dict(sim_time=7, rules=["SS-RES-3.1", "SS-RES-3.4"])


def p_ps_multicore():
    """PS with 2 cores, 3 closed users with think time 0.5: the state is the total
    number of jobs (one series, at the monitored replicaID), the utilisation is busy cores / cores,
    sampled at every resource event including stale ones."""
    m = single("ss_ps_multicore", [("internal", "work", [("CPU", "1.0")])], ("closed", 3, "0.5"),
               cpu=dict(id="cpu", type="CPU", policy="ProcessorSharing", rate="2.0", replicas=2))
    m.monitor(("resource", "cpu", 1), "state")
    return m, dict(sim_time=6, rules=["SS-RES-2.7", "SS-MEAS-3.1", "SS-MEAS-3.3", "SS-Q34"])


def p_zero_demand():
    """a zero demand on PS becomes a 1e-9 s job (JIFFY) with its own events and samples; on
    FCFS a zero demand is a job with service time 0."""
    m = single("ss_zero_demand", [("internal", "zero", [("CPU", "0.0")]), ("internal", "zero_hdd", [("HDD", "0")])],
               ("open", "1.0"), extra_specs=[dict(id="hdd", type="HDD", policy="FCFS", rate="1.0")])
    m.monitor(("resource", "hdd", 0), "state", "demand")
    return m, dict(sim_time=3, rules=["SS-RES-2.2", "SS-RES-3.4", "SS-MEAS-3.4"])


def p_delay_resource():
    """a demand on a Delay-scheduled resource has no simulated resource: JobAborted at
    initiation, ResourceDemandRequestAborted, UserAborted; the closed user restarts with a new User
    after a fresh think time."""
    m = single("ss_delay_resource", [("internal", "wait", [("DELAY", "0.5")])], ("closed", 1, "1.0"),
               cpu=dict(id="delay", type="DELAY", policy="Delay", rate="1.0"))
    return m, dict(sim_time=4, rules=["SS-RES-1.2", "SS-WL-5.1", "SS-Q25"])


def p_monitors():
    """response-time monitors on a usage scenario and an AssemblyOperationMeasuringPoint record;
    those on an EntryLevelSystemCallMeasuringPoint and a SystemOperationMeasuringPoint are ignored."""
    m = single("ss_monitors", [("internal", "work", [("CPU", "0.5")])], ("closed", 1, "0.5"), monitors=("scenario",))
    m.monitor(("assemblyop", "ac1", "prov1", "op"), "rt")
    m.monitor(("elsc", "ua2"), "rt")
    m.monitor(("sysop", "sysprov1", "op"), "rt")
    return m, dict(sim_time=4, rules=["SS-MEAS-2.2", "SS-Q29"])


def p_allocation_first():
    """one assembly context allocated twice (first on Fast with rate 2, then on Slow with rate
    1): Slingshot uses the first allocation context in list order for demands; SimuLizar the last."""
    m = Model("ss_allocation_first")
    m.container("rc_fast", "Fast", dict(id="cpu_fast", type="CPU", policy="ProcessorSharing", rate="2.0"))
    m.container("rc_slow", "Slow", dict(id="cpu_slow", type="CPU", policy="ProcessorSharing", rate="1.0"))
    m.interface("if1", "IService", "op")
    m.component("comp1", "Server", provides=[("prov1", "if1")], seffs=[("op", [("internal", "work", [("CPU", "1.0")])])])
    m.assembly("ac1", "Assembly_Server", "comp1")
    m.delegate("deleg1", "sysprov1", "ac1", "prov1", "if1")
    m.allocate("alc1", "ac1", "rc_fast")
    m.allocate("alc2", "ac1", "rc_slow")
    m.scenario("us1", "Scenario", ("closed", 1, "0.0"), [("call", "callOp", "sysprov1", "op")])
    m.monitor(("scenario", "us1"), "rt")
    m.monitor(("resource", "cpu_fast", 0), "state")
    m.monitor(("resource", "cpu_slow", 0), "state")
    return m, dict(sim_time=3, rules=["SS-ACT-9.1"])


def p_role_lookup():
    """one component (Front) assembled twice, FrontA wired to Fast, FrontB
    to Slow (connector A listed first). Call 1 enters a Client that calls FrontB through an assembly
    connector; FrontB's external call takes the first connector with that required role, which is A's:
    it reaches Fast, not Slow. Call 2 enters through a system role delegated to FrontB, but Slingshot
    takes the first assembly whose component provides the inner role: FrontA. Assembly-operation
    monitors: (FrontB, prov_front) records call 1; an entry-level SEFF is keyed by the system's outer
    role, so (FrontA, sysprov2) records call 2 and (FrontA, prov_front) records nothing."""
    m = Model("ss_role_lookup")
    m.container("rc1", "Node", dict(id="cpu", type="CPU", policy="ProcessorSharing", rate="1.0"))
    m.interface("if1", "IService", "op")
    m.interface("if2", "IBackend", "work")
    m.component("client", "Client", provides=[("prov_client", "if1")], requires=[("req_front", "if1")],
                seffs=[("op", [("external", "toFront", "req_front", "op")])])
    m.component("front", "Front", provides=[("prov_front", "if1")], requires=[("req_back", "if2")],
                seffs=[("op", [("internal", "pre", [("CPU", "0.1")]), ("external", "call", "req_back", "work")])])
    m.component("fast", "FastBackend", provides=[("prov_fast", "if2")], seffs=[("work", [("internal", "w", [("CPU", "0.2")])])])
    m.component("slow", "SlowBackend", provides=[("prov_slow", "if2")], seffs=[("work", [("internal", "w", [("CPU", "2.0")])])])
    m.assembly("acC", "Client", "client")
    m.assembly("acA", "FrontA", "front")
    m.assembly("acB", "FrontB", "front")
    m.assembly("acF", "Fast", "fast")
    m.assembly("acS", "Slow", "slow")
    m.connect("connA", "acA", "req_back", "acF", "prov_fast")
    m.connect("connB", "acB", "req_back", "acS", "prov_slow")
    m.connect("connC", "acC", "req_front", "acB", "prov_front")
    m.delegate("deleg1", "sysprov1", "acC", "prov_client", "if1")
    m.delegate("deleg2", "sysprov2", "acB", "prov_front", "if1")
    for ac in ("acC", "acA", "acB", "acF", "acS"):
        m.allocate("alc_" + ac, ac, "rc1")
    m.scenario("us1", "Scenario", ("closed", 1, "0.0"),
               [("call", "viaClient", "sysprov1", "op"), ("call", "direct", "sysprov2", "op")])
    m.monitor(("scenario", "us1"), "rt")
    m.monitor(("assemblyop", "acB", "prov_front", "op"), "rt")
    m.monitor(("assemblyop", "acA", "prov_front", "op"), "rt")
    m.monitor(("assemblyop", "acA", "sysprov2", "op"), "rt")
    m.monitor(("resource", "cpu", 0), "demand")
    return m, dict(sim_time=2, rules=["SS-ACT-1.2", "SS-ACT-2.2", "SS-MEAS-2.3", "SS-Q20", "SS-Q33"])


def p_link():
    """a remote call over a linking resource (latency 0.01, throughput 1000, BYTESIZE
    500 in, 250 back). Both directions are FCFS jobs on the link with demand BYTESIZE / throughput +
    latency; every link-job completion draws Math.random() for the failure check (tape site link:<id>)."""
    m = Model("ss_link")
    m.container("rc1", "Client", dict(id="cpu1", type="CPU", policy="ProcessorSharing", rate="1.0"))
    m.container("rc2", "Server", dict(id="cpu2", type="CPU", policy="ProcessorSharing", rate="1.0"))
    m.link("link1", "LAN", ["rc1", "rc2"], latency="0.01", throughput="1000.0")
    m.interface("if1", "IService", "op")
    m.interface("if2", "IRemote", "remote")
    m.component("client", "Client", provides=[("prov1", "if1")], requires=[("req1", "if2")],
                seffs=[("op", [("internal", "prepare", [("CPU", "0.1")]),
                               ("external", "callRemote", "req1", "remote", [("data", "BYTESIZE", "500")],
                                [("result", "BYTESIZE", "RETURN.BYTESIZE")])])])
    m.component("server", "RemoteServer", provides=[("prov2", "if2")],
                seffs=[("remote", [("internal", "serve", [("CPU", "0.2")]),
                                   ("setvar", "reply", [("RETURN", "BYTESIZE", "250")])])])
    m.assembly("ac1", "Assembly_Client", "client")
    m.assembly("ac2", "Assembly_Server", "server")
    m.connect("conn1", "ac1", "req1", "ac2", "prov2")
    m.delegate("deleg1", "sysprov1", "ac1", "prov1", "if1")
    m.allocate("alc1", "ac1", "rc1")
    m.allocate("alc2", "ac2", "rc2")
    m.scenario("us1", "Scenario", ("closed", 1, "0.5"), [("call", "callOp", "sysprov1", "op")])
    m.monitor(("scenario", "us1"), "rt")
    m.monitor(("assemblyop", "ac2", "prov2", "remote"), "rt")
    return m, dict(sim_time=3, rules=["SS-ACT-7", "SS-RES-5", "SS-RNG-1.2", "SS-Q28"])


def p_passive():
    """capacity-1 passive resource, 2 closed users: FIFO grants, the release re-evaluates its
    amount, waiting/holding time and passive state are recorded at AssemblyPassiveResourceMeasuringPoints
    (state sampled at PassiveResourceStateUpdated, a separate later event)."""
    m = single("ss_passive", [("acquire", "acquire", "pool", "1"), ("internal", "critical", [("CPU", "1.0")]),
                              ("release", "release", "pool", "1")],
               ("closed", 2, "0.5"), passive=[("pool", "pool", "1")])
    m.monitor(("passive", "ac1", "pool"), "wait", "hold", "pstate")
    return m, dict(sim_time=5, rules=["SS-RES-4", "SS-MEAS-4.2"])


def p_branches():
    """a usage-model Branch and a SEFF probabilistic BranchAction: one
    Math.random() draw each (tape sites branch:<id>), cumulative-probability selection; a guarded branch
    takes the first true guard."""
    m = single("ss_branches",
               [("branch", "pbranch", [("0.25", [("internal", "small", [("CPU", "0.1")])]),
                                       ("0.75", [("internal", "large", [("CPU", "0.3")])])]),
                ("guard", "gbranch", [("false", [("internal", "never", [("CPU", "9.0")])]),
                                      ("true", [("internal", "always", [("CPU", "0.05")])])])],
               ("open", "0.5"), monitors=("scenario",))
    m.scenarios[0]["steps"] = [("branch", "ubranch", [("0.5", [("call", "callA", "sysprov1", "op")]),
                                                      ("0.5", [("delay", "nap", "0.2")])])]
    m.monitor(("resource", "cpu", 0), "demand")
    return m, dict(sim_time=4, rules=["SS-ACT-5.1", "SS-WL-4.2", "SS-RNG-2"])


def p_guard_none():
    """a guarded BranchAction whose guards are all false throws
    IllegalStateException ("No branch transition was active"): the subscriber is disposed."""
    m = single("ss_guard_none", [("guard", "gbranch", [("false", [("internal", "a", [("CPU", "0.1")])])])],
               ("open", "1.0"))
    return m, dict(sim_time=4, rules=["SS-ACT-5.3", "SS-Q21", "SS-BUS-7"])


def p_usage_delay():
    """usage Delay -> UserSlept (delay 0) and UserWokeUp (delay d), draw at interpretation;
    the scenario response time includes the delay."""
    m = single("ss_usage_delay", [("internal", "work", [("CPU", "0.5")])], ("closed", 1, "0.25"))
    m.scenarios[0]["steps"] = [("delay", "nap", "Exp(4.0)"), ("call", "callOp", "sysprov1", "op")]
    return m, dict(sim_time=4, rules=["SS-WL-4.1"])


def p_async_fork():
    """a ForkAction with only asynchronous behaviours has no synchronisation point and throws
    IllegalStateException; with a synchronisation point, asynchronous behaviours are ignored."""
    m = single("ss_async_fork", [("fork", "fork", [[("internal", "sync", [("CPU", "0.2")])]],
                                  [[("internal", "async", [("CPU", "5.0")])]])],
               ("closed", 1, "0.5"))
    return m, dict(sim_time=4, rules=["SS-ACT-10.1"])


PROBES = [p_fifo_ties, p_two_demands, p_loop_zero, p_usage_loop_zero, p_stop_time, p_closed_think, p_open_draws,
          p_fcfs_replicas, p_ps_multicore, p_zero_demand, p_delay_resource, p_monitors, p_allocation_first,
          p_role_lookup, p_link, p_passive, p_branches, p_guard_none, p_usage_delay, p_async_fork]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(H, "models", "probes")
    only = set(sys.argv[2:])
    for p in PROBES:
        m, meta = p()
        if only and m.name not in only:
            continue
        d = os.path.join(out, m.name)
        m.write(d)
        meta = dict(name=m.name, seed=meta.get("seed", 1), sim_time=meta["sim_time"], rules=meta["rules"],
                    description=" ".join(p.__doc__.split()))
        with open(os.path.join(d, "probe.json"), "w") as f:
            json.dump(meta, f, indent=2)
            f.write("\n")
        print(d)


if __name__ == "__main__":
    main()
