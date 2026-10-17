//! The usage model (`docs/spec/slingshot/usage.md`, SS-WL): `UsageSimulationBehavior` and
//! `UsageScenarioInterpreter`.

use super::*;

impl Engine<'_> {
    /// `UserInterpretationContext.update()` with the given parts (the result frame is not
    /// carried over by `update()`).
    fn uctx(
        &self,
        c: &UCtx,
        action: Option<UserActionId>,
        beh: Rc<UBeh>,
        result: Option<SFrame>,
        user: Option<URef>,
    ) -> Rc<UCtx> {
        Rc::new(UCtx {
            scenario: c.scenario,
            action,
            user: user.unwrap_or_else(|| c.user.clone()),
            beh,
            result,
            closed: c.closed,
        })
    }

    /// The first `Start` of a scenario behaviour.
    fn first_start(&self, b: ScenarioBehaviourId) -> Option<UserActionId> {
        self.m.scenario_behaviours[b].start
    }

    /// `UsageScenarioBehaviorContext.startScenario()`.
    fn start_scenario(&self, b: &UBeh) -> Result<UserActionId, Throw> {
        let must = match b.kind {
            UBehKind::Root => b.start_once.get(),
            UBehKind::Loop => b.progression.get() < b.max,
            UBehKind::Branch => false,
        };
        if !must {
            return throw(
                "IllegalStateException",
                "This scenario cannot be repeated again",
            );
        }
        let Some(a) = self.first_start(b.behaviour) else {
            return throw(
                "IllegalStateException",
                "The model must have a Start action in order to perform",
            );
        };
        match b.kind {
            UBehKind::Root => b.start_once.set(false),
            UBehKind::Loop => b.progression.set(b.progression.get() + 1),
            UBehKind::Branch => {}
        }
        Ok(a)
    }

    fn root_beh(behaviour: ScenarioBehaviourId) -> Rc<UBeh> {
        Rc::new(UBeh {
            kind: UBehKind::Root,
            behaviour,
            next: None,
            parent: None,
            start_once: Cell::new(true),
            max: 0,
            progression: Cell::new(0),
        })
    }

    // --------------------------------------------------------------------------- handlers

    /// `onSimulationStart` (SS-WL-1.1).
    pub(super) fn on_simulation_start(&mut self) -> HR {
        let cm = self.cm;
        let mut out = Out::new();
        for (i, sc) in cm.scenarios.iter().enumerate() {
            let Some(beh) = sc.behaviour else {
                return throw(
                    "NullPointerException",
                    "usage scenario without a scenario behaviour",
                );
            };
            let Some(first) = self.first_start(beh) else {
                return throw(
                    "IllegalStateException",
                    "There must be a Start user action within the usage scenario.",
                );
            };
            let root = Self::root_beh(beh);
            match sc.workload {
                CWorkload::Closed { population, .. } => {
                    for _ in 0..population.max(0) {
                        let user = self.new_user();
                        let c = Rc::new(UCtx {
                            scenario: i,
                            action: Some(first),
                            user,
                            beh: root.clone(),
                            result: None,
                            closed: true,
                        });
                        out.extend(self.do_switch_user(&c, Some(first))?);
                    }
                }
                CWorkload::Open { .. } => {
                    let user = self.new_user();
                    let c = Rc::new(UCtx {
                        scenario: i,
                        action: Some(first),
                        user,
                        beh: root,
                        result: None,
                        closed: false,
                    });
                    out.extend(self.do_switch_user(&c, Some(first))?);
                }
            }
        }
        Ok(out)
    }

    /// `onUserStarted`: a new stack frame, then the current action.
    pub(super) fn on_user_started(&mut self, c: &Rc<UCtx>) -> HR {
        c.user.stack.borrow_mut().push(frame::new_frame(None));
        // (`UsageScenarioStarted` for the root behaviour: no subscriber)
        self.do_switch_user(c, c.action)
    }

    /// `onUserRequestFinished` (SS-WL-4.4).
    pub(super) fn on_user_request_finished(&mut self, r: &Rc<UserRequest>, c: &Rc<UCtx>) -> HR {
        if c.user.stack.borrow().len() != 2 {
            return throw("IllegalArgumentException", "null");
        }
        self.pop(&c.user)?;
        let top = self.top(&c.user)?;
        let cm = self.cm;
        if let CUAct::Elsc { outputs, .. } = &cm.uact[r.uact.index()] {
            self.add_parameters(c.result.as_ref(), outputs, &top, "return")?;
        }
        self.do_switch_user(c, c.action)
    }

    /// `onUserFinished` (SS-WL-3.1): pop the frame; a closed-workload user thinks.
    pub(super) fn on_user_finished(&mut self, c: &Rc<UCtx>) -> HR {
        self.pop(&c.user)?;
        let mut out = Out::new();
        if c.closed {
            let think = self.think_time(c)?;
            out.push((Ev::ClosedInit(c.clone()), delay(think)?));
        }
        // (`UsageScenarioFinished`: no subscriber)
        Ok(out)
    }

    fn think_time(&mut self, c: &UCtx) -> Result<f64, Throw> {
        let CWorkload::Closed { think_time, .. } = self.cm.scenarios[c.scenario].workload else {
            return throw("NullPointerException", "no think time");
        };
        self.eval_f64(think_time, None, "think")
    }

    /// `onUserAborted` (SS-WL-5.1): a closed-workload user is replaced by a new one.
    pub(super) fn on_user_aborted(&mut self, c: &Rc<UCtx>) -> HR {
        let mut out = Out::new();
        if c.closed {
            let user = self.new_user();
            let n = self.uctx(c, c.action, c.beh.clone(), None, Some(user));
            let think = self.think_time(&n)?;
            out.push((Ev::ClosedInit(n), delay(think)?));
        }
        Ok(out)
    }

    /// `onClosedWorkloadUserInitiated` (SS-WL-3.2).
    pub(super) fn on_closed_initiated(&mut self, c: &Rc<UCtx>) -> HR {
        let root = Self::root_beh(c.beh.behaviour);
        let n = self.uctx(c, c.action, root.clone(), None, None);
        let a = self.start_scenario(&root)?;
        self.do_switch_user(&n, Some(a))
    }

    // --------------------------------------------------------------------------- interpreter

    /// `UsageScenarioInterpreter.doSwitch(action)`: the case's events after the passed element.
    pub(super) fn do_switch_user(&mut self, c: &Rc<UCtx>, a: Option<UserActionId>) -> HR {
        let Some(a) = a else {
            return throw(
                "IllegalArgumentException",
                "called interpretation on a null reference",
            );
        };
        let rest = self.user_case(c, a)?;
        let mut out = Vec::with_capacity(rest.len() + 1);
        out.push((Ev::UPassed(c.clone(), a), 0.0));
        out.extend(rest);
        Ok(out)
    }

    fn user_case(&mut self, c: &Rc<UCtx>, a: UserActionId) -> HR {
        let cm = self.cm;
        let m = self.m;
        let ua = &m.user_actions[a];
        match &ua.kind {
            UserActionKind::Start => {
                if c.beh.kind != UBehKind::Root {
                    let n = self.uctx(c, ua.successor, c.beh.clone(), None, None);
                    return Ok(vec![(Ev::UProgressed(n), 0.0)]);
                }
                if c.closed {
                    let n = self.uctx(c, ua.successor, c.beh.clone(), None, None);
                    return Ok(vec![(Ev::UserStarted(n), 0.0)]);
                }
                // open workload: the next arrival is drawn now (SS-WL-2.1)
                let CWorkload::Open { inter_arrival } = cm.scenarios[c.scenario].workload else {
                    return throw(
                        "IllegalStateException",
                        "The user must be a open workload or closed workload user",
                    );
                };
                let ia = self.eval_f64(inter_arrival, None, "interarrival")?;
                let beh = cm.scenarios[c.scenario]
                    .behaviour
                    .expect("scenario behaviour");
                let Some(first) = self.first_start(beh) else {
                    return throw(
                        "IllegalStateException",
                        "There must be a Start user action within the usage scenario.",
                    );
                };
                let user = self.new_user();
                let next = Rc::new(UCtx {
                    scenario: c.scenario,
                    action: Some(first),
                    user,
                    beh: Self::root_beh(beh),
                    result: None,
                    closed: false,
                });
                let n = self.uctx(c, ua.successor, c.beh.clone(), None, None);
                Ok(vec![
                    (Ev::UserStarted(n), 0.0),
                    (Ev::InterArrival(next), delay(ia)?),
                ])
            }
            UserActionKind::Stop => {
                if c.beh.kind != UBehKind::Root {
                    if c.beh.kind == UBehKind::Loop && c.beh.progression.get() < c.beh.max {
                        let s = self.start_scenario(&c.beh)?;
                        return self.do_switch_user(c, Some(s));
                    }
                    let parent = c.beh.parent.clone().expect("child context with a parent");
                    let Some(next) = c.beh.next else {
                        return throw("NoSuchElementException", "No value present");
                    };
                    let n = self.uctx(c, next, parent, None, None);
                    return Ok(vec![(Ev::UProgressed(n), 0.0)]);
                }
                Ok(vec![(Ev::UserFinished(c.clone()), 0.0)])
            }
            UserActionKind::EntryLevelSystemCall {
                role, signature, ..
            } => {
                let r = Rc::new(UserRequest {
                    user: c.user.clone(),
                    uact: a,
                    role: *role,
                    sig: *signature,
                });
                let n = self.uctx(
                    c,
                    ua.successor,
                    c.beh.clone(),
                    Some(frame::new_frame(None)),
                    None,
                );
                Ok(vec![(Ev::EntryReq(r, n), 0.0)])
            }
            UserActionKind::Delay { .. } => {
                let CUAct::Delay(p) = cm.uact[a.index()] else {
                    unreachable!()
                };
                let d = self.eval_f64(p, None, "delay")?;
                let n = self.uctx(c, ua.successor, c.beh.clone(), None, None);
                // (`UserSlept` at delay 0: no subscriber)
                Ok(vec![(Ev::WokeUp(n), delay(d)?)])
            }
            UserActionKind::Loop { body, .. } => {
                let CUAct::Loop { count, .. } = cm.uact[a.index()] else {
                    unreachable!()
                };
                let n = self.eval_i32(count, None, "loop")?;
                if ua.successor.is_none() {
                    return throw("NullPointerException", "Optional.of(null): loop successor");
                }
                let Some(body) = *body else {
                    return throw("IllegalArgumentException", "null");
                };
                if n <= 0 {
                    return throw(
                        "IllegalArgumentException",
                        format!("InitialLoopCount must be in [0,{n}] (maximalLoopCount) but is 0"),
                    );
                }
                let beh = Rc::new(UBeh {
                    kind: UBehKind::Loop,
                    behaviour: body,
                    next: Some(ua.successor),
                    parent: Some(c.beh.clone()),
                    start_once: Cell::new(false),
                    max: n,
                    progression: Cell::new(0),
                });
                let first = self.start_scenario(&beh)?;
                let nc = self.uctx(c, Some(first), beh, None, None);
                Ok(vec![(Ev::InnerInit(nc), 0.0)])
            }
            UserActionKind::Branch { transitions } => {
                let top = self.top(&c.user)?;
                drop(top);
                let CUAct::Branch { cum, .. } = &cm.uact[a.index()] else {
                    unreachable!()
                };
                let i = self.random_index(cum, &ua.id)?;
                let Some(t) = i.and_then(|i| transitions.get(i)) else {
                    return throw(
                        "ArrayIndexOutOfBoundsException",
                        format!("Index -1 out of bounds for length {}", transitions.len()),
                    );
                };
                let Some(b) = t.behaviour else {
                    return throw(
                        "NullPointerException",
                        "branch transition without behaviour",
                    );
                };
                let Some(first) = self.first_start(b) else {
                    return throw(
                        "IllegalStateException",
                        "There is no start action within the branch transition!",
                    );
                };
                if ua.successor.is_none() {
                    return throw(
                        "NullPointerException",
                        "Optional.of(null): branch successor",
                    );
                }
                let beh = Rc::new(UBeh {
                    kind: UBehKind::Branch,
                    behaviour: b,
                    next: Some(ua.successor),
                    parent: Some(c.beh.clone()),
                    start_once: Cell::new(false),
                    max: 0,
                    progression: Cell::new(0),
                });
                let nc = self.uctx(c, Some(first), beh, None, None);
                Ok(vec![(Ev::InnerInit(nc), 0.0)])
            }
        }
    }

    /// `TransitionDeterminer.getRandomIndex`: one `Math.random()` at `branch:<id>`; `None` for
    /// an empty list (no draw) or no index.
    pub(super) fn random_index(&mut self, cum: &[f64], id: &str) -> Result<Option<usize>, Throw> {
        let Some(&last) = cum.last() else {
            return Ok(None);
        };
        let mut site = String::with_capacity(id.len() + 7);
        site.push_str("branch:");
        site.push_str(id);
        let r = self.rng.math_random(&site);
        Ok(cum.iter().position(|&s| last * r < s))
    }
}
