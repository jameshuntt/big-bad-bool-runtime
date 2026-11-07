//! Registration: programs, routes, injections and the runtime observer, checked against the
//! bounds once, at `build`.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use big_bad_bool::program::{NoObserver, Observer, Stepper, VerifiedProgram};

use crate::config::{AdmissionPolicy, RuntimeConfig};
use crate::effect::{EffectTable, ResolvedTable, TableError};
use crate::ids::{Port, ProgramId, RouteId, RunId};
use crate::message::RuntimeMessage;
use crate::observer::{NoRuntimeObserver, RuntimeObserver};

/// One program to register: a verified program, its effect table, its step observer and,
/// optionally, its own admission policy.
///
/// `build` makes the program's `Stepper` and the instance owns it from then on; the run moves it
/// into the program's own Scope task. No API hands a stepper or its state out before the run
/// ends.
pub struct ProgramInstance<'p, O> {
    pub(crate) program: &'p VerifiedProgram,
    pub(crate) table: EffectTable,
    pub(crate) observer: O,
    pub(crate) policy: Option<AdmissionPolicy>,
}

impl<'p> ProgramInstance<'p, NoObserver> {
    /// `program` with no step observer, its effects turned into runtime effects by `table`, under
    /// the runtime's admission policy.
    #[must_use]
    pub const fn new(program: &'p VerifiedProgram, table: EffectTable) -> Self {
        Self {
            program,
            table,
            observer: NoObserver,
            policy: None,
        }
    }
}

impl<'p, O: Observer> ProgramInstance<'p, O> {
    /// The same program with another step observer. Step observers see the program's step events
    /// by shared reference and return nothing; they see committed steps only (`prepare` does not
    /// call them), so a step admission refused is never shown to them.
    #[must_use]
    pub fn observer<O2: Observer>(self, observer: O2) -> ProgramInstance<'p, O2> {
        ProgramInstance {
            program: self.program,
            table: self.table,
            observer,
            policy: self.policy,
        }
    }

    /// The same program under its own admission policy instead of the runtime's.
    #[must_use]
    pub fn policy(mut self, policy: AdmissionPolicy) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// `from`'s `port` delivers to `to`. Several routes on one port fan out, in the order added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Route {
    /// The sender.
    pub from: ProgramId,
    /// The sender's port.
    pub port: Port,
    /// The receiver.
    pub to: ProgramId,
}

impl Route {
    /// `from`'s `port` delivers to `to`.
    #[must_use]
    pub const fn new(from: ProgramId, port: Port, to: ProgramId) -> Self {
        Self { from, port, to }
    }

    /// The route's durable id ([`RouteId::of`]).
    #[must_use]
    pub fn id(&self) -> RouteId {
        RouteId::of(self.from, self.port, self.to)
    }
}

/// Why `build` refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// More programs than `RuntimeLimits::programs`.
    TooManyPrograms {
        /// Registered.
        observed: usize,
        /// The bound.
        permitted: usize,
    },
    /// More routes than `RuntimeLimits::routes`.
    TooManyRoutes {
        /// Declared.
        observed: usize,
        /// The bound.
        permitted: usize,
    },
    /// Two programs registered under one id.
    DuplicateProgram(ProgramId),
    /// A route or an injection names an unregistered program.
    UnknownProgram(ProgramId),
    /// More routes on one `(program, port)` than `RuntimeLimits::fanout`.
    FanOut {
        /// The sender.
        from: ProgramId,
        /// The port.
        port: Port,
        /// Routes declared.
        observed: usize,
        /// The bound.
        permitted: usize,
    },
    /// A `Retry` policy asks for a larger backlog than `RuntimeLimits::retry_backlog`.
    Backlog {
        /// The program (`None`: the runtime's own policy).
        program: Option<ProgramId>,
        /// The backlog asked for.
        requested: usize,
        /// The bound.
        permitted: usize,
    },
    /// A program's effect table does not fit its spec.
    EffectTable {
        /// The program.
        program: ProgramId,
        /// What is wrong.
        error: TableError,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyPrograms {
                observed,
                permitted,
            } => write!(f, "{observed} programs registered, {permitted} permitted"),
            Self::TooManyRoutes {
                observed,
                permitted,
            } => write!(f, "{observed} routes declared, {permitted} permitted"),
            Self::DuplicateProgram(id) => write!(f, "program {id} registered twice"),
            Self::UnknownProgram(id) => write!(f, "program {id} is not registered"),
            Self::FanOut {
                from,
                port,
                observed,
                permitted,
            } => write!(
                f,
                "{observed} routes from {from} port {}, {permitted} permitted",
                port.0
            ),
            Self::Backlog {
                program,
                requested,
                permitted,
            } => match program {
                Some(p) => write!(
                    f,
                    "program {p} asks for a retry backlog of {requested}, {permitted} permitted"
                ),
                None => write!(
                    f,
                    "the runtime policy asks for a retry backlog of {requested}, {permitted} permitted"
                ),
            },
            Self::EffectTable { program, error } => {
                write!(f, "effect table of {program}: {error:?}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// Collects programs, routes, injections and the runtime observer. Made by [`Runtime::builder`].
pub struct RuntimeBuilder<'p, O, W> {
    config: RuntimeConfig,
    programs: Vec<(ProgramId, ProgramInstance<'p, O>)>,
    routes: Vec<Route>,
    injections: Vec<(u32, ProgramId, RuntimeMessage)>,
    observer: W,
}

/// A built program: its stepper, owned from `build` on, and its resolved table and policy.
pub(crate) struct Instance<'p, O: Observer> {
    pub(crate) program: &'p VerifiedProgram,
    pub(crate) stepper: Stepper<'p, O>,
    pub(crate) table: ResolvedTable,
    pub(crate) policy: AdmissionPolicy,
}

/// A built runtime: programs, steppers, routes and effect tables fixed, bounds checked.
/// [`Runtime::run`] runs it on Scope.
pub struct Runtime<'p, O: Observer, W = NoRuntimeObserver> {
    pub(crate) config: RuntimeConfig,
    pub(crate) run_id: RunId,
    pub(crate) ids: Vec<ProgramId>,
    pub(crate) instances: Vec<Instance<'p, O>>,
    /// Per program slot: `(port, receiver slots in route order)`.
    pub(crate) routes: Vec<Vec<(Port, Vec<usize>)>>,
    pub(crate) route_ids: Vec<RouteId>,
    /// `(epoch, receiver slot, message)`, epochs ascending, then in the order added.
    pub(crate) injections: Vec<(u32, usize, RuntimeMessage)>,
    pub(crate) observer: W,
}

impl<'p, O: Observer> Runtime<'p, O, NoRuntimeObserver> {
    /// A builder under `config`, with no runtime observer.
    #[must_use]
    pub fn builder(config: RuntimeConfig) -> RuntimeBuilder<'p, O, NoRuntimeObserver> {
        RuntimeBuilder {
            config,
            programs: Vec::new(),
            routes: Vec::new(),
            injections: Vec::new(),
            observer: NoRuntimeObserver,
        }
    }
}

impl<O: Observer, W> Runtime<'_, O, W> {
    /// The registered program ids, by slot (the order their Scope tasks are spawned).
    #[must_use]
    pub fn program_ids(&self) -> &[ProgramId] {
        &self.ids
    }

    /// The run's id: [`RunId::of_topology`] of one line `program <id> <spec digest>` per program
    /// in slot order, then one line `route <route id>` per route in the order added. The same
    /// named topology gives the same id; configuration and effect tables are not part of it.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Every route's id, in the order added.
    #[must_use]
    pub fn route_ids(&self) -> &[RouteId] {
        &self.route_ids
    }

    /// The configuration it was built with.
    #[must_use]
    pub const fn config(&self) -> &RuntimeConfig {
        &self.config
    }
}

impl<'p, O: Observer, W: RuntimeObserver> RuntimeBuilder<'p, O, W> {
    /// Registers `instance` under `id`.
    #[must_use]
    pub fn program(mut self, id: ProgramId, instance: ProgramInstance<'p, O>) -> Self {
        self.programs.push((id, instance));
        self
    }

    /// Adds a route.
    #[must_use]
    pub fn route(mut self, route: Route) -> Self {
        self.routes.push(route);
        self
    }

    /// Queues `message` for `to` before the first scheduler round (epoch 0).
    #[must_use]
    pub fn inject(self, to: ProgramId, message: RuntimeMessage) -> Self {
        self.inject_in_epoch(0, to, message)
    }

    /// Queues `message` for `to` at the start of epoch `epoch`: after every earlier epoch ran
    /// until nothing was runnable and no timer waited.
    #[must_use]
    pub fn inject_in_epoch(mut self, epoch: u32, to: ProgramId, message: RuntimeMessage) -> Self {
        self.injections.push((epoch, to, message));
        self
    }

    /// The same builder with a runtime observer, which sees every runtime event and changes
    /// nothing ([`RuntimeObserver`]).
    #[must_use]
    pub fn observer<W2: RuntimeObserver>(self, observer: W2) -> RuntimeBuilder<'p, O, W2> {
        RuntimeBuilder {
            config: self.config,
            programs: self.programs,
            routes: self.routes,
            injections: self.injections,
            observer,
        }
    }

    /// Checks every bound that does not depend on the run, resolves every effect table, makes
    /// every program's stepper, then fixes the runtime.
    ///
    /// # Errors
    ///
    /// [`BuildError`] for too many programs or routes, a duplicate id, an unknown program in a
    /// route or an injection, too many routes on one port, a retry backlog past its bound, or an
    /// effect table that does not fit.
    pub fn build(self) -> Result<Runtime<'p, O, W>, BuildError> {
        let limits = self.config.limits;
        if self.programs.len() > limits.programs {
            return Err(BuildError::TooManyPrograms {
                observed: self.programs.len(),
                permitted: limits.programs,
            });
        }
        if self.routes.len() > limits.routes {
            return Err(BuildError::TooManyRoutes {
                observed: self.routes.len(),
                permitted: limits.routes,
            });
        }
        let check_backlog = |program: Option<ProgramId>, policy: AdmissionPolicy| match policy {
            AdmissionPolicy::Retry { backlog, .. } if backlog > limits.retry_backlog => {
                Err(BuildError::Backlog {
                    program,
                    requested: backlog,
                    permitted: limits.retry_backlog,
                })
            }
            _ => Ok(()),
        };
        check_backlog(None, self.config.policy)?;
        let mut index = BTreeMap::new();
        for (i, (id, instance)) in self.programs.iter().enumerate() {
            if index.insert(id.0, i).is_some() {
                return Err(BuildError::DuplicateProgram(*id));
            }
            if let Some(policy) = instance.policy {
                check_backlog(Some(*id), policy)?;
            }
        }
        let find = |id: ProgramId| {
            index
                .get(&id.0)
                .copied()
                .ok_or(BuildError::UnknownProgram(id))
        };
        let mut routes: Vec<Vec<(Port, Vec<usize>)>> = vec![Vec::new(); self.programs.len()];
        for r in &self.routes {
            let (from, to) = (find(r.from)?, find(r.to)?);
            let ports = &mut routes[from];
            match ports.iter_mut().find(|(p, _)| *p == r.port) {
                Some((_, targets)) => targets.push(to),
                None => ports.push((r.port, vec![to])),
            }
        }
        for (from, ports) in routes.iter().enumerate() {
            for (port, targets) in ports {
                if targets.len() > limits.fanout {
                    return Err(BuildError::FanOut {
                        from: self.programs[from].0,
                        port: *port,
                        observed: targets.len(),
                        permitted: limits.fanout,
                    });
                }
            }
        }
        let tables = self
            .programs
            .iter()
            .map(|(id, instance)| {
                ResolvedTable::resolve(instance.program.spec(), &instance.table).map_err(|error| {
                    BuildError::EffectTable {
                        program: *id,
                        error,
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut injections = self
            .injections
            .into_iter()
            .map(|(epoch, to, m)| Ok((epoch, find(to)?, m)))
            .collect::<Result<Vec<_>, BuildError>>()?;
        // Stable: within an epoch, the order added.
        injections.sort_by_key(|(epoch, _, _)| *epoch);
        let route_ids: Vec<RouteId> = self.routes.iter().map(Route::id).collect();
        let mut topology = String::new();
        for (id, instance) in &self.programs {
            let _ = writeln!(
                topology,
                "program {id} {}",
                instance.program.digest().as_str()
            );
        }
        for r in &route_ids {
            let _ = writeln!(topology, "route {r}");
        }
        let run_id = RunId::of_topology(&topology);
        let default_policy = self.config.policy;
        let mut ids = Vec::with_capacity(self.programs.len());
        let instances = self
            .programs
            .into_iter()
            .zip(tables)
            .map(|((id, instance), table)| {
                ids.push(id);
                Instance {
                    program: instance.program,
                    stepper: Stepper::new(instance.program, instance.observer),
                    table,
                    policy: instance.policy.unwrap_or(default_policy),
                }
            })
            .collect();
        Ok(Runtime {
            config: self.config,
            run_id,
            ids,
            instances,
            routes,
            route_ids,
            injections,
            observer: self.observer,
        })
    }
}
