//! System entry, calls and SEFF interpretation (`docs/spec/slingshot/seff.md`, SS-ACT):
//! `SystemSimulationBehavior`, `SeffSimulationBehavior`, `SeffInterpreter`,
//! `RepositoryInterpreter`, `SystemModelRepositoryImpl` and the behaviour context holders.

use super::*;

/// What `SeffBehaviorContextHolder.getNextAction()` returned.
enum Next {
    Action(ActionId),
    /// An `InfrastructureCall` of an infrastructure-calls holder.
    Infra(ActionId, usize),
}

/// The arguments of a `RepositoryInterpreter`.
struct Repo {
    ac: AssemblyContextId,
    sig: Option<SignatureId>,
    role: Option<RoleId>,
    user: URef,
    caller: Option<Rc<SeffCtx>>,
    cow: Option<Rc<Cow>>,
    result: Option<SFrame>,
    uctx: Option<Rc<UCtx>>,
    request: Option<Rc<UserRequest>>,
}

/// The providing assembly context and provided role of an assembly connector.
type ConnEnds = (Option<AssemblyContextId>, Option<RoleId>);

const NO_NEXT: &str = "There is no action after StopAction, or a successor action was not defined.";

impl Engine<'_> {
    fn role_id(&self, r: Option<RoleId>) -> Option<&str> {
        r.map(|r| &*self.m.roles[r].id)
    }

    /// `SystemModelRepositoryImpl.findAssemblyContextFromRequiredRole` and
    /// `findProvidedRoleFromRequiredRole`: the first assembly connector of the system with that
    /// required role id (SS-ACT-2.2, Q20).
    fn assembly_connector(&self, role: Option<RoleId>) -> Result<Option<ConnEnds>, Throw> {
        let m = self.m;
        let Some(rid) = self.role_id(role) else {
            return throw("NullPointerException", "required role is null");
        };
        for &c in &m.structures[self.sys].connectors {
            if let ConnectorKind::Assembly {
                providing,
                required_role,
                provided_role,
                ..
            } = &m.connectors[c].kind
            {
                let Some(r) = required_role else {
                    return throw(
                        "NullPointerException",
                        "assembly connector without required role",
                    );
                };
                if *m.roles[*r].id == *rid {
                    return Ok(Some((*providing, *provided_role)));
                }
            }
        }
        Ok(None)
    }

    fn component_provides(&self, comp: ComponentId, rid: &str) -> bool {
        self.m.components[comp]
            .provided_roles
            .iter()
            .any(|&r| *self.m.roles[r].id == *rid)
    }

    // --------------------------------------------------------------------------- holders

    /// `new SeffBehaviorWrapper(behaviour)`: the cursor at the first `StartAction`.
    fn wrapper(&self, beh: Option<BehaviourId>) -> Result<Wrapper, Throw> {
        let Some(b) = beh else {
            return throw("NullPointerException", "behaviour is null");
        };
        match self.m.behaviours[b].start {
            Some(s) => Ok(Wrapper {
                beh: b,
                current: Some(s),
                finished: false,
            }),
            None => throw(
                "IllegalStateException",
                "This model does not have a start action.",
            ),
        }
    }

    fn holder(
        &self,
        kind: HKind,
        behaviours: &[Option<BehaviourId>],
        successor: Option<Option<ActionId>>,
        child: bool,
    ) -> Result<HRef, Throw> {
        if let Some(None) = successor {
            return throw("NullPointerException", "Optional.of(null): successor");
        }
        let mut wrappers = Vec::with_capacity(behaviours.len());
        for &b in behaviours {
            wrappers.push(self.wrapper(b)?);
        }
        Ok(Rc::new(RefCell::new(Holder {
            kind,
            wrappers,
            child,
        })))
    }

    fn has_finished(h: &Holder) -> bool {
        match &h.kind {
            HKind::Infra { calls, pos } => *pos >= calls.len(),
            HKind::Loop { max, progression } => {
                h.wrappers.iter().all(|w| w.finished) && progression == max
            }
            _ => h.wrappers.iter().all(|w| w.finished),
        }
    }

    /// `getCurrentProcessedBehavior()`: the wrapper index (the fork holder moves on to the next
    /// unfinished behaviour at every call, SS-ACT-10.2).
    fn current_processed(h: &mut Holder) -> Result<usize, Throw> {
        match &mut h.kind {
            HKind::Infra { .. } => throw(
                "IllegalStateException",
                "InfrastructureSegmentContextHolder has no current processed behaviour, as InfraCalls are modelled as a simple list.",
            ),
            HKind::Fork {
                unfinished,
                cursor,
                last_ret,
                current,
                ..
            } => {
                let r = *current;
                // updateCurrentBehaviorIndex(): an ArrayList iterator over the unfinished ones
                if *cursor == unfinished.len() {
                    *cursor = 0;
                    *last_ret = None;
                }
                let next =
                    |unfinished: &Vec<usize>, cursor: &mut usize, last_ret: &mut Option<usize>| {
                        if *cursor >= unfinished.len() {
                            return throw("NoSuchElementException", "null");
                        }
                        let i = *cursor;
                        *cursor += 1;
                        *last_ret = Some(i);
                        Ok(unfinished[i])
                    };
                *current = next(unfinished, cursor, last_ret)?;
                while h.wrappers[*current].finished {
                    let l = last_ret.take().expect("iterator remove after next");
                    unfinished.remove(l);
                    *cursor = l;
                    *current = next(unfinished, cursor, last_ret)?;
                }
                Ok(r)
            }
            _ => Ok(0),
        }
    }

    /// `SeffBehaviorWrapper.next()`.
    fn wrapper_next(&self, w: &mut Wrapper) -> Result<ActionId, Throw> {
        if w.finished {
            return throw("NoSuchElementException", NO_NEXT);
        }
        let Some(a) = w.current else {
            return throw("NoSuchElementException", NO_NEXT);
        };
        w.finished = matches!(self.m.actions[a].kind, ActionKind::Stop);
        w.current = self.m.actions[a].successor;
        Ok(a)
    }

    /// `getNextAction()` of a holder.
    fn next_action(&self, h: &HRef) -> Result<Next, Throw> {
        let mut h = h.borrow_mut();
        if let HKind::Infra { calls, pos } = &mut h.kind {
            // Iterator.next()
            let Some(&c) = calls.get(*pos) else {
                return throw("NoSuchElementException", "null");
            };
            *pos += 1;
            return Ok(Next::Infra(c.0, c.1));
        }
        if let HKind::Loop { max, progression } = &mut h.kind {
            let (max, prog) = (*max, *progression);
            if h.wrappers[0].finished && prog < max {
                if let HKind::Loop { progression, .. } = &mut h.kind {
                    *progression += 1;
                }
                // repeatScenario()
                let start = self.m.behaviours[h.wrappers[0].beh].start;
                h.wrappers[0].current = start;
                h.wrappers[0].finished = false;
            }
        }
        if Self::has_finished(&h) {
            return throw(
                "NoSuchElementException",
                "There is no action to interpret anymore, as this model has finished.",
            );
        }
        let w = Self::current_processed(&mut h)?;
        let a = self.wrapper_next(&mut h.wrappers[w])?;
        Ok(Next::Action(a))
    }

    // --------------------------------------------------------------------------- contexts

    /// `getCurrentResultStackframe()`.
    fn result_frame(&self, c: &SeffCtx) -> Result<SFrame, Throw> {
        if let Some(f) = &c.result {
            return Ok(f.clone());
        }
        match &c.parent {
            Some(p) => self.result_frame(p),
            None => self.top(&c.rpc.user),
        }
    }

    /// `update().build()`: a copy with the result frame resolved now.
    fn ctx_update(&mut self, c: &SeffCtx) -> Result<Rc<SeffCtx>, Throw> {
        let result = Some(self.result_frame(c)?);
        Ok(Rc::new(SeffCtx {
            id: self.ctx_id(),
            holder: c.holder.clone(),
            rpc: c.rpc.clone(),
            ac: c.ac,
            caller: c.caller.clone(),
            cow: c.cow.clone(),
            parent: c.parent.clone(),
            result,
        }))
    }

    /// `createChildContext()...build()`.
    fn child_ctx(
        &mut self,
        c: &Rc<SeffCtx>,
        holder: HRef,
        caller: Option<Rc<SeffCtx>>,
    ) -> Rc<SeffCtx> {
        Rc::new(SeffCtx {
            id: self.ctx_id(),
            holder,
            rpc: c.rpc.clone(),
            ac: c.ac,
            caller,
            cow: None,
            parent: Some(c.clone()),
            result: None,
        })
    }

    /// `InfrastructureCallsContextHolder(context, action, parent)`: every `numberOfCalls` is
    /// evaluated now, in list order (SS-ACT-2.5).
    fn infra_holder(&mut self, c: &SeffCtx, a: ActionId) -> Result<HRef, Throw> {
        let cm = self.cm;
        let CAct::Internal { infra, .. } = &cm.act[a.index()] else {
            unreachable!()
        };
        let mut calls = Vec::new();
        for (i, ic) in infra.iter().enumerate() {
            let top = self.top(&c.rpc.user)?;
            let n = self.eval_i32(ic.count, Some(&top), "infra")?;
            for _ in 0..n.max(0) {
                calls.push((a, i));
            }
        }
        Ok(Rc::new(RefCell::new(Holder {
            kind: HKind::Infra { calls, pos: 0 },
            wrappers: Vec::new(),
            child: true,
        })))
    }

    // --------------------------------------------------------------------------- system

    /// `onUserEntryRequested` (SS-ACT-1).
    pub(super) fn on_user_entry_requested(&mut self, r: &Rc<UserRequest>, uc: &Rc<UCtx>) -> HR {
        let m = self.m;
        let cm = self.cm;
        let Some(rid) = self.role_id(r.role) else {
            return throw("NullPointerException", "entry call without role");
        };
        // getConnectedProvidedDelegationConnector
        if !m.systems[self.system]
            .provided_roles
            .iter()
            .any(|&x| *m.roles[x].id == *rid)
        {
            return Ok(Out::new());
        }
        let conn =
            m.structures[self.sys]
                .connectors
                .iter()
                .find_map(|&c| match &m.connectors[c].kind {
                    ConnectorKind::ProvidedDelegation {
                        outer_role: Some(o),
                        inner_role,
                        ..
                    } if *m.roles[*o].id == *rid => Some(*inner_role),
                    _ => None,
                });
        let Some(inner) = conn else {
            return Ok(Out::new());
        };
        let Some(inner) = inner else {
            return throw(
                "NullPointerException",
                "delegation connector without inner role",
            );
        };
        let iid = &*m.roles[inner].id;
        let Some(sig) = r.sig else {
            return throw("NullPointerException", "entry call without signature");
        };
        let sid = &*m.signatures[sig].id;
        // getDelegatedComponentSeff: first SEFF of the first basic component providing the role
        let acs = &m.structures[self.sys].assembly_contexts;
        let seff = acs.iter().any(|&ac| {
            m.assembly_contexts[ac].component.is_some_and(|comp| {
                self.component_provides(comp, iid)
                    && matches!(m.components[comp].kind, ComponentKind::Basic { .. })
                    && m.components[comp].seffs().iter().any(|&s| {
                        m.seffs[s]
                            .signature
                            .is_some_and(|x| *m.signatures[x].id == *sid)
                    })
            })
        });
        // findAssemblyContextByProvidedRole: the first assembly whose component provides it
        let ac = acs.iter().copied().find(|&ac| {
            m.assembly_contexts[ac]
                .component
                .is_some_and(|comp| self.component_provides(comp, iid))
        });
        let (true, Some(ac)) = (seff, ac) else {
            return Ok(Out::new());
        };
        let CUAct::Elsc { inputs, .. } = &cm.uact[r.uact.index()] else {
            unreachable!()
        };
        self.push_new_frame(&r.user, inputs, None, "param")?;
        let ri = Repo {
            ac,
            sig: Some(sig),
            role: r.role,
            user: r.user.clone(),
            caller: None,
            cow: None,
            result: Some(frame::new_frame(None)),
            uctx: Some(uc.clone()),
            request: Some(r.clone()),
        };
        let comp = m.assembly_contexts[ac].component;
        let ctxs = self.repo_component(&ri, comp)?;
        Ok(ctxs
            .into_iter()
            .map(|c| (Ev::SProgressed(c), 0.0))
            .collect())
    }

    /// `RepositoryInterpreter.doSwitch(component)`.
    fn repo_component(
        &mut self,
        ri: &Repo,
        comp: Option<ComponentId>,
    ) -> Result<Vec<Rc<SeffCtx>>, Throw> {
        let m = self.m;
        let cm = self.cm;
        let Some(comp) = comp else {
            return throw(
                "IllegalArgumentException",
                "called interpretation on a null reference",
            );
        };
        match &m.components[comp].kind {
            ComponentKind::Basic { seffs, .. } => {
                // caseBasicComponent: component parameter frame and assembly frame (SS-ACT-1.3)
                let parent = self.top(&ri.user)?;
                let cf = self.push_new_frame(
                    &ri.user,
                    &cm.comp_params[comp.index()],
                    Some(parent),
                    "component",
                )?;
                self.push_new_frame(
                    &ri.user,
                    &cm.ac_params[ri.ac.index()],
                    Some(cf),
                    "component",
                )?;
                let Some(sig) = ri.sig else {
                    return throw("NullPointerException", "signature is null");
                };
                let sid = &*m.signatures[sig].id;
                let mut out = Vec::new();
                for &s in seffs {
                    let Some(ss) = m.seffs[s].signature else {
                        return throw("NullPointerException", "SEFF without described service");
                    };
                    if *m.signatures[ss].id != *sid {
                        continue;
                    }
                    let holder =
                        self.holder(HKind::Root, &[Some(m.seffs[s].behaviour)], None, false)?;
                    let rpc = Rc::new(Rpc {
                        role: ri.role,
                        user: ri.user.clone(),
                        uctx: ri.uctx.clone(),
                        request: ri.request.clone(),
                    });
                    out.push(Rc::new(SeffCtx {
                        id: self.ctx_id(),
                        holder,
                        rpc,
                        ac: ri.ac,
                        caller: ri.caller.clone(),
                        cow: ri.cow.clone(),
                        parent: None,
                        result: ri.result.clone(),
                    }));
                }
                Ok(out)
            }
            ComponentKind::Composite { structure } | ComponentKind::SubSystem { structure } => {
                // caseComposedProvidingRequiringEntity: only for the role's own providing entity
                let Some(role) = ri.role else {
                    return throw("NullPointerException", "provided role is null");
                };
                if m.roles[role].owner != RoleOwner::Component(comp) {
                    return Ok(Vec::new());
                }
                let rid = &*m.roles[role].id;
                let conn = m.structures[*structure].connectors.iter().find_map(|&c| {
                    match &m.connectors[c].kind {
                        ConnectorKind::ProvidedDelegation {
                            outer_role: Some(o),
                            inner_role,
                            assembly,
                        } if *m.roles[*o].id == *rid => Some((*inner_role, *assembly)),
                        _ => None,
                    }
                });
                let Some((Some(inner), Some(ac))) = conn else {
                    return throw("IllegalStateException", "null");
                };
                let ri2 = Repo {
                    ac,
                    sig: ri.sig,
                    role: Some(inner),
                    user: ri.user.clone(),
                    caller: None,
                    cow: ri.cow.clone(),
                    result: ri.result.clone(),
                    uctx: None,
                    request: None,
                };
                // caseProvidedRole: the role's providing entity
                let owner = match m.roles[inner].owner {
                    RoleOwner::Component(c) => Some(c),
                    _ => None,
                };
                self.repo_component(&ri2, owner)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// `requestCallOverWire` (`onRequestInitiated`, SS-ACT-2.3).
    pub(super) fn request_call_over_wire(&mut self, g: &Rc<Ger>) -> HR {
        let cm = self.cm;
        let Some((Some(to), Some(_prov))) = self.assembly_connector(g.role)? else {
            return Ok(Out::new());
        };
        let inputs: &[CChar] = match g.kind {
            GerKind::External(a) => match &cm.act[a.index()] {
                CAct::External { inputs, .. } => inputs,
                _ => &[],
            },
            GerKind::Infra(a, i) => match &cm.act[a.index()] {
                CAct::Internal { infra, .. } => &infra[i].inputs,
                _ => &[],
            },
        };
        let f = self.push_new_frame(&g.user, inputs, None, "param")?;
        let cow = Rc::new(Cow {
            from: g.from.ac,
            to,
            user: g.user.clone(),
            entry: g.clone(),
            vars: Some(f),
            reply: false,
        });
        Ok(vec![(Ev::CowReq(cow), 0.0)])
    }

    /// `onCallOverWireSucceeded` (SS-ACT-2.3, SS-ACT-2.4).
    pub(super) fn on_cow_succeeded(&mut self, cow: &Rc<Cow>) -> HR {
        let cm = self.cm;
        let m = self.m;
        let g = &cow.entry;
        if cow.reply {
            self.pop(&g.user)?;
            let top = self.top(&g.user)?;
            let returns: &[CChar] = match g.kind {
                GerKind::External(a) => match &cm.act[a.index()] {
                    CAct::External { returns, .. } => returns,
                    _ => &[],
                },
                GerKind::Infra(..) => &[],
            };
            self.add_parameters(cow.vars.as_ref(), returns, &top, "return")?;
            return Ok(vec![(Ev::SProgressed(g.from.clone()), 0.0)]);
        }
        if let Some((Some(ac), Some(prov))) = self.assembly_connector(g.role)? {
            let ri = Repo {
                ac,
                sig: g.sig,
                role: Some(prov),
                user: g.user.clone(),
                caller: Some(g.from.clone()),
                cow: Some(cow.clone()),
                result: Some(frame::new_frame(None)),
                uctx: None,
                request: None,
            };
            let comp = m.assembly_contexts[ac].component;
            let ctxs = self.repo_component(&ri, comp)?;
            return Ok(ctxs
                .into_iter()
                .map(|c| (Ev::SProgressed(c), 0.0))
                .collect());
        }
        match &g.from.rpc.uctx {
            Some(u) => Ok(vec![(Ev::UserAborted(u.clone()), 0.0)]),
            None => throw("NullPointerException", "UserAborted(null)"),
        }
    }

    /// `onCallOverWireAborted` (SS-ACT-7.3): pop the input frame and request again.
    pub(super) fn on_cow_aborted(&mut self, cow: &Rc<Cow>) -> HR {
        self.pop(&cow.user)?;
        self.request_call_over_wire(&cow.entry)
    }

    /// `onResourceDemandRequestAborted` (SS-WL-5.1, Q30).
    pub(super) fn on_demand_aborted(&mut self, r: &Rc<Rdr>) -> HR {
        let mut c = r.ctx.clone();
        loop {
            if let Some(u) = &c.rpc.uctx {
                return Ok(vec![(Ev::UserAborted(u.clone()), 0.0)]);
            }
            c = match (&c.caller, &c.parent) {
                (Some(x), _) => x.clone(),
                (None, Some(p)) => p.clone(),
                (None, None) => {
                    return throw(
                        "NoSuchElementException",
                        "User Interpretation Context not found!",
                    );
                }
            };
        }
    }

    /// `onSEFFInfrastructureCalled` (SS-ACT-2.5): no input frame (Q36), no transfer.
    pub(super) fn on_infra_called(&mut self, g: &Rc<Ger>) -> HR {
        let m = self.m;
        let Some(rid) = self.role_id(g.role) else {
            return throw("NullPointerException", "infrastructure call without role");
        };
        let ac =
            m.structures[self.sys]
                .connectors
                .iter()
                .find_map(|&c| match &m.connectors[c].kind {
                    ConnectorKind::AssemblyInfrastructure {
                        providing,
                        required_role: Some(r),
                        ..
                    } if *m.roles[*r].id == *rid => Some(*providing),
                    _ => None,
                });
        let Some(ac) = ac else {
            return Ok(Out::new());
        };
        let Some(ac) = ac else {
            return throw(
                "NullPointerException",
                "infrastructure connector without assembly",
            );
        };
        let ri = Repo {
            ac,
            sig: g.sig,
            role: None,
            user: g.user.clone(),
            caller: Some(g.from.clone()),
            cow: None,
            result: None,
            uctx: None,
            request: None,
        };
        let comp = m.assembly_contexts[ac].component;
        let ctxs = self.repo_component(&ri, comp)?;
        Ok(ctxs
            .into_iter()
            .map(|c| (Ev::SProgressed(c), 0.0))
            .collect())
    }

    /// `onActiveResourceFinished` (SS-ACT-3.2, Q17): the cursor's predecessor decides whether
    /// infrastructure calls follow.
    pub(super) fn on_active_finished(&mut self, r: &Rc<Rdr>) -> HR {
        let m = self.m;
        let pc = &r.ctx;
        let cur = {
            let mut h = pc.holder.borrow_mut();
            let w = Self::current_processed(&mut h)?;
            h.wrappers[w].current
        };
        let Some(cur) = cur else {
            return throw(
                "NullPointerException",
                "Cannot invoke \"org.palladiosimulator.pcm.seff.AbstractAction.getPredecessor_AbstractAction()\" because the return value of \"org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.entities.seff.behaviorcontext.SeffBehaviorWrapper.getCurrentAction()\" is null",
            );
        };
        let pred = m.actions[cur].predecessor;
        let infra = pred.filter(|&p| {
            matches!(m.actions[p].kind, ActionKind::Internal)
                && !m.actions[p].infrastructure_calls.is_empty()
        });
        let Some(p) = infra else {
            return Ok(vec![(Ev::SProgressed(pc.clone()), 0.0)]);
        };
        {
            let mut h = pc.holder.borrow_mut();
            Self::current_processed(&mut h)?;
        }
        let holder = self.infra_holder(pc, p)?;
        let child = Rc::new(SeffCtx {
            id: self.ctx_id(),
            holder,
            rpc: pc.rpc.clone(),
            ac: pc.ac,
            caller: pc.caller.clone(),
            cow: None,
            parent: None,
            result: None,
        });
        Ok(vec![(Ev::SProgressed(child), 0.0)])
    }

    // --------------------------------------------------------------------------- SEFF behaviour

    /// `onSeffInterpretationProgressed`.
    pub(super) fn on_seff_progressed(&mut self, c: &Rc<SeffCtx>) -> HR {
        let infra_done = {
            let h = c.holder.borrow();
            matches!(h.kind, HKind::Infra { .. }) && Self::has_finished(&h)
        };
        if infra_done {
            return match &c.parent {
                Some(p) => Ok(vec![(Ev::SProgressed(p.clone()), 0.0)]),
                None => throw(
                    "IllegalStateException",
                    "Every child context must have a parent",
                ),
            };
        }
        let n = self.next_action(&c.holder)?;
        self.seff_switch(c, n)
    }

    /// `onPassiveResourceAcquired` (SS-ACT-8.2).
    pub(super) fn on_passive_acquired(&mut self, r: &Rc<Rdr>) -> HR {
        let n = self.next_action(&r.ctx.holder)?;
        self.seff_switch(&r.ctx, n)
    }

    /// `onSEFFChildInterpretationStarted`.
    pub(super) fn on_seff_child_started(&mut self, c: &Rc<SeffCtx>) -> HR {
        let n = self.next_action(&c.holder)?;
        self.seff_switch(c, n)
    }

    /// `onSEFFInterpretationFinished` (SS-ACT-4.2).
    pub(super) fn on_seff_finished(&mut self, c: &Rc<SeffCtx>) -> HR {
        let (is_fork, finished, child) = {
            let h = c.holder.borrow();
            (
                matches!(h.kind, HKind::Fork { .. }),
                Self::has_finished(&h),
                h.child,
            )
        };
        let parent = || match &c.parent {
            Some(p) => Ok(vec![(Ev::SProgressed(p.clone()), 0.0)]),
            None => throw(
                "IllegalStateException",
                "Every child context must have a parent",
            ),
        };
        if is_fork {
            if !finished {
                return Ok(Out::new());
            }
            let mut h = c.holder.borrow_mut();
            if let HKind::Fork { processed, .. } = &mut h.kind {
                if *processed {
                    return Ok(Out::new());
                }
                *processed = true;
            }
            drop(h);
            return parent();
        }
        if !finished {
            return Ok(vec![(Ev::SProgressed(c.clone()), 0.0)]);
        }
        if child {
            return parent();
        }
        if let Some(caller) = &c.caller {
            // continueInCaller: pop the component frames, then reply over the wire or continue
            self.pop(&c.rpc.user)?;
            self.pop(&c.rpc.user)?;
            return match &c.cow {
                Some(cow) => {
                    let vars = Some(self.result_frame(c)?);
                    let reply = Rc::new(Cow {
                        from: cow.from,
                        to: cow.to,
                        user: cow.user.clone(),
                        entry: cow.entry.clone(),
                        vars,
                        reply: true,
                    });
                    Ok(vec![(Ev::CowReq(reply), 0.0)])
                }
                None => Ok(vec![(Ev::SProgressed(caller.clone()), 0.0)]),
            };
        }
        // finishUserRequest
        let Some(uc) = c.rpc.uctx.clone() else {
            return throw("NullPointerException", "UserInterpretationContext is null");
        };
        let rf = self.result_frame(c)?;
        let nu = Rc::new(UCtx {
            scenario: uc.scenario,
            action: uc.action,
            user: uc.user.clone(),
            beh: uc.beh.clone(),
            result: Some(rf),
            closed: uc.closed,
        });
        self.pop(&c.rpc.user)?;
        self.pop(&c.rpc.user)?;
        match &c.rpc.request {
            Some(r) => Ok(vec![(Ev::ReqFinished(r.clone(), nu), 0.0)]),
            None => throw("NullPointerException", "UserRequestFinished(null)"),
        }
    }

    /// `SeffInterpreter.doSwitch(action)`.
    fn seff_switch(&mut self, c: &Rc<SeffCtx>, n: Next) -> HR {
        let m = self.m;
        let cm = self.cm;
        let a = match n {
            Next::Infra(a, i) => {
                // caseCallAction of an InfrastructureCall
                let ic = &m.actions[a].infrastructure_calls[i];
                let from = self.ctx_update(c)?;
                let g = Rc::new(Ger {
                    kind: GerKind::Infra(a, i),
                    role: ic.role,
                    sig: ic.signature,
                    user: c.rpc.user.clone(),
                    from,
                });
                return Ok(vec![(Ev::InfraCalled(g), 0.0)]);
            }
            Next::Action(a) => a,
        };
        let act = &m.actions[a];
        match &act.kind {
            ActionKind::Start => Ok(vec![
                (Ev::SProgressed(c.clone()), 0.0),
                (Ev::SPassed(c.clone(), a), 0.0),
            ]),
            ActionKind::Stop => Ok(vec![
                (Ev::SFinished(c.clone()), 0.0),
                (Ev::SPassed(c.clone(), a), 0.0),
            ]),
            ActionKind::Branch { transitions } => {
                if transitions.is_empty() {
                    return throw(
                        "IllegalStateException",
                        "Empty branch action is not allowed!",
                    );
                }
                let top = self.top(&c.rpc.user)?;
                let chosen = match &cm.act[a.index()] {
                    CAct::ProbBranch { cum, .. } => {
                        if transitions
                            .iter()
                            .any(|t| !matches!(t.condition, BranchCondition::Probability(_)))
                        {
                            return throw(
                                "ClassCastException",
                                "GuardedBranchTransition cannot be cast to ProbabilisticBranchTransition",
                            );
                        }
                        match self.random_index(cum, &act.id)? {
                            Some(i) => i,
                            None => {
                                return throw(
                                    "IndexOutOfBoundsException",
                                    format!(
                                        "Index -1 out of bounds for length {}",
                                        transitions.len()
                                    ),
                                );
                            }
                        }
                    }
                    CAct::GuardBranch { guards, .. } => {
                        let mut found = None;
                        for (i, t) in transitions.iter().enumerate() {
                            if !matches!(t.condition, BranchCondition::Guard(_)) {
                                return throw(
                                    "ClassCastException",
                                    "ProbabilisticBranchTransition cannot be cast to GuardedBranchTransition",
                                );
                            }
                            if self.eval_bool(guards[i], Some(&top), "guard")? {
                                found = Some(i);
                                break;
                            }
                        }
                        match found {
                            Some(i) => i,
                            None => {
                                return throw(
                                    "IllegalStateException",
                                    "No branch transition was active. This is not allowed.",
                                );
                            }
                        }
                    }
                    _ => unreachable!(),
                };
                Self::current_processed(&mut c.holder.borrow_mut())?;
                let holder = self.holder(
                    HKind::Branch,
                    &[transitions[chosen].behaviour],
                    Some(act.successor),
                    true,
                )?;
                let child = self.child_ctx(c, holder, None);
                Ok(vec![(Ev::SChild(child), 0.0)])
            }
            ActionKind::Loop { body, .. } => {
                let CAct::Loop { count, .. } = cm.act[a.index()] else {
                    unreachable!()
                };
                let top = self.top(&c.rpc.user)?;
                let n = self.eval_i32(count, Some(&top), "loop")?;
                Self::current_processed(&mut c.holder.borrow_mut())?;
                let holder = self.holder(
                    HKind::Loop {
                        max: n,
                        progression: 1,
                    },
                    &[*body],
                    Some(act.successor),
                    true,
                )?;
                if n < 0 {
                    return throw("IllegalArgumentException", "The loop counter must be >= 0");
                }
                let child = self.child_ctx(c, holder, c.caller.clone());
                Ok(vec![(Ev::SChild(child), 0.0)])
            }
            ActionKind::CollectionIterator { body, .. } => {
                let CAct::Collection { count, prefix, .. } = &cm.act[a.index()] else {
                    unreachable!()
                };
                let top = self.top(&c.rpc.user)?;
                let n = self.eval_i32(*count, Some(&top), "collection")?;
                // a frame for the INNER values (parent: the top frame), never popped (Q22)
                let inner = frame::new_frame(Some(top));
                c.rpc.user.stack.borrow_mut().push(inner.clone());
                // evaluateInner(frame, name + "."): getContents() of the topmost frame
                for (k, b) in frame::contents(&inner) {
                    if !cm.keys.names[k as usize].starts_with(&**prefix) {
                        continue;
                    }
                    if let Binding::Proxy(p) = b {
                        let prev = std::mem::replace(&mut self.rng.site, "collection");
                        let v = crate::frames::eval_proxy(cm, &p, &mut self.rng);
                        self.rng.site = prev;
                        let v = v.map_err(|e| Throw {
                            class: "StochasticExpressionEvaluationFailedException",
                            message: format!("{e:?}"),
                        })?;
                        inner.borrow_mut().put(k, Binding::Val(v));
                    }
                }
                Self::current_processed(&mut c.holder.borrow_mut())?;
                let holder = self.holder(
                    HKind::Loop {
                        max: n,
                        progression: 1,
                    },
                    &[*body],
                    Some(act.successor),
                    true,
                )?;
                if n < 0 {
                    return throw("IllegalArgumentException", "The loop counter must be >= 0");
                }
                // createChildContextPrefilled(): update(), parent = this, no result frame
                self.result_frame(c)?;
                let child = Rc::new(SeffCtx {
                    id: self.ctx_id(),
                    holder,
                    rpc: c.rpc.clone(),
                    ac: c.ac,
                    caller: c.caller.clone(),
                    cow: c.cow.clone(),
                    parent: Some(c.clone()),
                    result: None,
                });
                Ok(vec![(Ev::SChild(child), 0.0)])
            }
            ActionKind::Fork {
                synchronisation, ..
            } => {
                let Some(sp) = synchronisation else {
                    return throw(
                        "IllegalStateException",
                        "ForkAction must have a synchronisation point!",
                    );
                };
                if sp.synchronous.is_empty() {
                    return throw(
                        "IllegalStateException",
                        "Empty forked behaviors is not allowed",
                    );
                }
                Self::current_processed(&mut c.holder.borrow_mut())?;
                let bs: Vec<Option<BehaviourId>> =
                    sp.synchronous.iter().map(|&b| Some(b)).collect();
                let holder = self.holder(
                    HKind::Fork {
                        unfinished: (0..bs.len()).collect(),
                        cursor: 1,
                        last_ret: Some(0),
                        current: 0,
                        processed: false,
                    },
                    &bs,
                    Some(act.successor),
                    true,
                )?;
                let mut out = Out::new();
                for _ in &bs {
                    let child = self.child_ctx(c, holder.clone(), c.caller.clone());
                    out.push((Ev::SChild(child), 0.0));
                }
                Ok(out)
            }
            ActionKind::ExternalCall {
                role, signature, ..
            } => {
                let from = self.ctx_update(c)?;
                let g = Rc::new(Ger {
                    kind: GerKind::External(a),
                    role: *role,
                    sig: *signature,
                    user: c.rpc.user.clone(),
                    from,
                });
                Ok(vec![(Ev::ExtCalled(g), 0.0)])
            }
            ActionKind::Acquire { resource, .. } | ActionKind::Release { resource } => {
                let acquire = matches!(act.kind, ActionKind::Acquire { .. });
                if act.resource_demands.is_empty() {
                    return throw(
                        "NoSuchElementException",
                        if acquire {
                            "No parametric resource demand specified for AcquireAction!"
                        } else {
                            "No parametric resource demand specified for ReleaseAction!"
                        },
                    );
                }
                let r = Rc::new(Rdr {
                    ac: c.ac,
                    ctx: c.clone(),
                    passive: *resource,
                    active: false,
                    action: a,
                    demand: 0,
                });
                if acquire {
                    Ok(vec![(Ev::RdReq(r), 0.0)])
                } else {
                    Ok(vec![(Ev::PRel(r), 0.0), (Ev::SProgressed(c.clone()), 0.0)])
                }
            }
            ActionKind::SetVariable { .. } => {
                let CAct::SetVariable(usages) = &cm.act[a.index()] else {
                    unreachable!()
                };
                let top = self.top(&c.rpc.user)?;
                let rf = self.result_frame(c)?;
                self.add_parameters(Some(&top), usages, &rf, "setvar")?;
                Ok(vec![(Ev::SProgressed(c.clone()), 0.0)])
            }
            ActionKind::Internal => {
                let mut out = Out::new();
                for i in 0..act.resource_demands.len() {
                    let r = Rc::new(Rdr {
                        ac: c.ac,
                        ctx: c.clone(),
                        passive: None,
                        active: true,
                        action: a,
                        demand: i,
                    });
                    out.push((Ev::RdReq(r), 0.0));
                }
                if out.is_empty() && !act.infrastructure_calls.is_empty() {
                    Self::current_processed(&mut c.holder.borrow_mut())?;
                    let holder = self.infra_holder(c, a)?;
                    let child = self.child_ctx(c, holder, c.caller.clone());
                    out.push((Ev::SProgressed(child), 0.0));
                }
                if out.is_empty() {
                    out.push((Ev::SProgressed(c.clone()), 0.0));
                }
                Ok(out)
            }
            // InternalCallAction, EmitEventAction (caseCallAction: ignored), RecoveryAction and
            // others: no event, the request stops (Q35)
            _ => Ok(Out::new()),
        }
    }
}
