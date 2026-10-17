package simoxide.slingshot.instrument;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import javax.measure.Measure;

import org.eclipse.emf.ecore.EObject;
import org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation.entities.jobs.ActiveJob;
import org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation.entities.jobs.Job;
import org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation.entities.jobs.LinkingJob;
import org.palladiosimulator.analyzer.slingshot.behavior.spd.data.ModelAdjustmentRequested;
import org.palladiosimulator.analyzer.slingshot.behavior.spd.data.SimulationTimeReached;
import org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.entities.GeneralEntryRequest;
import org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.entities.resource.ResourceDemandRequest;
import org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.entities.seff.SEFFInterpretationContext;
import org.palladiosimulator.analyzer.slingshot.behavior.usagemodel.entities.User;
import org.palladiosimulator.analyzer.slingshot.behavior.usagemodel.entities.UserRequest;
import org.palladiosimulator.analyzer.slingshot.behavior.usagemodel.entities.interpretationcontext.UserInterpretationContext;
import org.palladiosimulator.analyzer.slingshot.common.events.AbstractEntityChangedEvent;
import org.palladiosimulator.analyzer.slingshot.common.events.DESEvent;
import org.palladiosimulator.analyzer.slingshot.common.events.modelchanges.ModelAdjusted;
import org.palladiosimulator.analyzer.slingshot.common.events.modelchanges.ModelChange;
import org.palladiosimulator.analyzer.slingshot.common.events.modelchanges.ResourceEnvironmentChange;
import org.palladiosimulator.analyzer.slingshot.common.utils.events.ModelPassedEvent;
import org.palladiosimulator.analyzer.slingshot.monitor.data.entities.SlingshotMeasuringValue;
import org.palladiosimulator.pcm.resourceenvironment.ResourceContainer;

/**
 * Per-run instrumentation state of the SimOxide Slingshot reference runner. The runner bundle calls
 * {@link #begin(Options)} before the driver is initialised and {@link #end()} after the run; in between,
 * {@link InstrumentBehavior} (a Slingshot behaviour extension) feeds it every dispatched event.
 *
 * Outputs (each optional) in the run directory:
 * <ul>
 * <li>{@code trace.jsonl}: one line per dispatched DES event, in dispatch order: event type, simulated
 * time and stable entity keys (PCM ids; users and jobs numbered in order of first appearance; no
 * UUIDs, no identity hashes).</li>
 * <li>{@code tape.jsonl}: every uniform draw of the two random streams ({@code stoex}, {@code math}) with
 * the index of the event during which it was drawn.</li>
 * <li>{@code spd.jsonl}: SPD decision log (see {@link SpdObserver}).</li>
 * </ul>
 * Nothing here may compute identity hash codes or call {@code toString()} on model objects: that would
 * change Slingshot's identity-hash-dependent orders in unpatched mode.
 */
public final class Instrument {

	public static final class Options {
		public Path dir;
		public boolean trace = true;
		public boolean tape = true;
		public boolean spd = true;
		/** SPD evaluations to log: "all", or "decisions" (skip events the target-group check rejects). */
		public String spdLevel = "decisions";
		public String header = "{}";
	}

	private static volatile Instrument run;

	public static void begin(final Options o) throws IOException {
		run = new Instrument(o);
	}

	/** Closes the run's files; returns a one-line JSON summary of what was written. */
	public static String end() throws IOException {
		final Instrument r = run;
		run = null;
		if (r == null) {
			return "{}";
		}
		return r.close();
	}

	public static boolean active() {
		return run != null;
	}

	static Instrument current() {
		return run;
	}

	/** One uniform draw of stream {@code s} ("stoex" or "math"); no-op when not recording. */
	public static void tape(final String s, final double u) {
		final Instrument r = run;
		if (r != null && r.tape != null) {
			r.tapeDraw(s, u);
		}
	}

	// ---------------------------------------------------------------------------------------------

	final Options opt;
	private final BufferedWriter trace;
	private final BufferedWriter tape;
	final SpdObserver spd;
	private final Map<String, Integer> users = new HashMap<>();
	private final Map<String, Integer> jobs = new HashMap<>();
	private Object currentEvent;
	long eventIndex = -1;
	double now;
	private long tapeStoex, tapeMath, measurementsMade;
	String error;

	private Instrument(final Options o) throws IOException {
		this.opt = o;
		Files.createDirectories(o.dir);
		this.trace = o.trace ? writer("trace.jsonl") : null;
		this.tape = o.tape ? writer("tape.jsonl") : null;
		this.spd = o.spd ? new SpdObserver(this, writer("spd.jsonl")) : null;
		if (trace != null) {
			trace.write("{\"ev\":\"header\",\"t\":0.0,\"format\":\"slingshot-trace/1\",\"run\":" + o.header + "}\n");
		}
	}

	private BufferedWriter writer(final String name) throws IOException {
		return Files.newBufferedWriter(opt.dir.resolve(name), StandardCharsets.UTF_8);
	}

	private String close() throws IOException {
		if (trace != null || tape != null) {
			writeSubscribers();
		}
		if (spd != null) {
			spd.close();
		}
		if (trace != null) {
			trace.write(new Json().str("ev", "finish").num("t", now).integer("events", eventIndex + 1)
					.integer("measurements_made", measurementsMade).integer("stoex", tapeStoex)
					.integer("math", tapeMath).end());
			trace.write('\n');
			trace.close();
		}
		if (tape != null) {
			tape.close();
		}
		return new Json().integer("events", eventIndex + 1).integer("measurements_made", measurementsMade)
				.integer("stoex_draws", tapeStoex).integer("math_draws", tapeMath)
				.integer("spd_evaluations", spd == null ? 0 : spd.evaluations)
				.integer("spd_requests", spd == null ? 0 : spd.requests).str("error", error).end();
	}

	void fail(final Throwable t) {
		if (error == null) {
			error = t.getClass().getName() + ": " + t.getMessage();
			t.printStackTrace();
		}
	}

	private void tapeDraw(final String s, final double u) {
		final boolean math = "math".equals(s);
		final long i = math ? tapeMath++ : tapeStoex++;
		final String o = math ? mathSite() : stoexSite();
		try {
			tape.write(new Json().str("k", "u").str("s", s).integer("i", i).num("u", u).integer("e", eventIndex)
					.str("o", o).end());
			tape.write('\n');
		} catch (final IOException e) {
			throw new UncheckedIOException(e);
		}
	}

	// ---------------------------------------------------------------------------------------------
	// events

	/** Called by every pre-interception; the first call for a new event object is its dispatch. */
	void dispatched(final DESEvent e) throws IOException {
		if (e == currentEvent) {
			return;
		}
		currentEvent = e;
		eventIndex++;
		now = e.time();
		if (e instanceof AbstractEntityChangedEvent<?> ec && ec.getEntity() instanceof SlingshotMeasuringValue) {
			measurementsMade++;
		}
		if (trace == null) {
			return;
		}
		final Json j = new Json().str("ev", e.getName()).num("t", e.time());
		describe(e, j);
		trace.write(j.end());
		trace.write('\n');
	}

	private void describe(final DESEvent e, final Json j) {
		if (e instanceof final ModelPassedEvent<?> m) {
			element(j, m.getModelElement());
			if (e instanceof final org.palladiosimulator.analyzer.slingshot.behavior.usagemodel.events.UsageModelPassedElement<?> u) {
				user(j, u.getContext().getUser());
			} else if (e instanceof final org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.events.SEFFModelPassedElement<?> s) {
				seff(j, s.getContext());
			}
			return;
		}
		if (e instanceof final SimulationTimeReached s) {
			j.str("tg", s.getTargetGroupId());
			return;
		}
		if (e instanceof final ModelAdjustmentRequested m) {
			j.str("policy", m.getScalingPolicy().getId());
			return;
		}
		if (e instanceof final ModelAdjusted a) {
			j.bool("ok", a.isWasSuccessful()).integer("changes", a.getChanges().size());
			for (final ModelChange<?> c : a.getChanges()) {
				if (c instanceof final ResourceEnvironmentChange rc) {
					j.raw("new", names(rc.getNewResourceContainers()));
					j.raw("deleted", names(rc.getDeletedResourceContainers()));
				}
			}
			return;
		}
		if (!(e instanceof final AbstractEntityChangedEvent<?> ec)) {
			return;
		}
		final Object o = ec.getEntity();
		if (o instanceof final UserInterpretationContext c) {
			user(j, c.getUser());
			element(j, c.getCurrentAction());
		} else if (o instanceof final UserRequest r) {
			user(j, r.getUser());
			j.str("role", Keys.id(r.getOperationProvidedRole())).str("sig", Keys.id(r.getOperationSignature()));
		} else if (o instanceof final SEFFInterpretationContext c) {
			seff(j, c);
		} else if (o instanceof final GeneralEntryRequest g) {
			j.str("role", Keys.id(g.getRequiredRole())).str("sig", Keys.id(g.getSignature()));
			seff(j, g.getRequestFrom());
		} else if (o instanceof final ResourceDemandRequest r) {
			user(j, r.getUser());
			j.str("ac", Keys.id(r.getAssemblyContext())).str("kind", String.valueOf(r.getResourceType()));
			r.getPassiveResource().ifPresent(p -> j.str("passive", Keys.id(p)));
			if (r.getParametricResourceDemand() != null) {
				j.str("res", Keys.id(r.getParametricResourceDemand().getRequiredResource_ParametricResourceDemand()));
				j.str("action", Keys.id(r.getParametricResourceDemand().getAction_ParametricResourceDemand()));
			}
		} else if (o instanceof final Job job) {
			j.integer("job", seq(jobs, job.getId())).num("d", job.getDemand());
			if (job instanceof final ActiveJob a) {
				j.str("rc", Keys.id(a.getAllocationContext().getResourceContainer_AllocationContext()))
						.str("res", Keys.id(a.getProcessingResourceType()));
				if (a.getRequest() != null) {
					user(j, a.getRequest().getUser());
				}
			} else if (job instanceof final LinkingJob l) {
				j.str("link", Keys.id(l.getLinkingResource()));
			}
		} else if (o instanceof final SlingshotMeasuringValue mv) {
			j.str("mp", Keys.mpKey(mv.getMeasuringPoint())).str("metric", mv.getMetricDesciption().getName());
			j.raw("v", row(mv));
		} else if (o instanceof final EObject eo) {
			element(j, eo);
		}
	}

	/** Tuple as [time, value...] in SimOxide's normalised order. */
	static String row(final SlingshotMeasuringValue mv) {
		final List<Measure<?, ?>> l = mv.asList();
		final int ti = Keys.timeIndex(mv.getMetricDesciption());
		final double[] row = new double[l.size()];
		for (int i = 0, k = 1; i < row.length; i++) {
			final int pos = i == ti ? 0 : (ti < 0 ? i : k++);
			row[pos] = ((Number) l.get(i).getValue()).doubleValue();
		}
		final StringBuilder sb = new StringBuilder("[");
		for (int i = 0; i < row.length; i++) {
			if (i > 0) {
				sb.append(',');
			}
			Json.num(sb, row[i]);
		}
		return sb.append(']').toString();
	}

	private static String names(final List<ResourceContainer> l) {
		final StringBuilder sb = new StringBuilder("[");
		for (final ResourceContainer c : l) {
			if (sb.length() > 1) {
				sb.append(',');
			}
			Json.quote(sb, c.getEntityName() == null ? "" : c.getEntityName());
		}
		return sb.append(']').toString();
	}

	private static void element(final Json j, final EObject el) {
		if (el != null) {
			j.str("type", el.eClass().getName()).str("el", Keys.id(el));
		}
	}

	private void user(final Json j, final User u) {
		if (u != null) {
			j.integer("u", seq(users, u.getId()));
		}
	}

	private void seff(final Json j, final SEFFInterpretationContext c) {
		if (c == null) {
			return;
		}
		if (c.getRequestProcessingContext() != null) {
			user(j, c.getRequestProcessingContext().getUser());
		}
		j.str("ac", Keys.id(c.getAssemblyContext()));
	}

	private static int seq(final Map<String, Integer> m, final String id) {
		Integer n = m.get(id);
		if (n == null) {
			n = m.size() + 1;
			m.put(id, n);
		}
		return n;
	}

	// ---------------------------------------------------------------------------------------------
	// draw sites (tape "o")

	private static final String SLINGSHOT = "org.palladiosimulator.analyzer.slingshot.";
	private static final StackWalker WALKER = StackWalker.getInstance();
	private static final java.lang.invoke.MethodHandle TAKE_SITE = takeSite();

	private static java.lang.invoke.MethodHandle takeSite() {
		try {
			final Class<?> c = ClassLoader.getSystemClassLoader().loadClass("simoxide.slingshot.seed.Sites");
			return java.lang.invoke.MethodHandles.publicLookup().findStatic(c, "take",
					java.lang.invoke.MethodType.methodType(Object.class));
		} catch (final ReflectiveOperationException | RuntimeException e) {
			return null;
		}
	}

	/**
	 * Site of a {@code Math.random()} draw, announced by the seed agent's hooks ({@code Sites}):
	 * {@code branch:<id>} for the usage Branch or SEFF BranchAction whose transitions were passed to
	 * {@code TransitionDeterminer}, {@code link:<id>} for the failure draw of a linking resource, {@code ?}
	 * otherwise.
	 */
	private static String mathSite() {
		Object site = null;
		try {
			site = TAKE_SITE == null ? null : (Object) TAKE_SITE.invokeExact();
		} catch (final Throwable t) {
			return "?";
		}
		if (site instanceof final List<?> l && !l.isEmpty() && l.get(0) instanceof final EObject t
				&& t.eContainer() != null) {
			return "branch:" + Keys.id(t.eContainer());
		}
		if (site != null && site.getClass().getSimpleName().equals("SimulatedLinkingResource")) {
			try {
				return "link:" + Keys.id((EObject) site.getClass().getMethod("getLinkingResource").invoke(site));
			} catch (final ReflectiveOperationException | RuntimeException e) {
				return "link:?";
			}
		}
		return "?";
	}

	/**
	 * Site of a StoEx draw: {@code Class.method} of the innermost Slingshot frame on the stack, skipping the
	 * generic helpers {@code AbstractStoExProxy} and {@code SimulatedStackHelper} (their caller says why the
	 * expression is evaluated). Classes outside Slingshot (StoEx, SimuCom variables) are skipped too.
	 */
	private static String stoexSite() {
		return WALKER.walk(st -> st.filter(f -> f.getClassName().startsWith(SLINGSHOT))
				.filter(f -> !f.getClassName().endsWith(".AbstractStoExProxy")
						&& !f.getClassName().endsWith(".SimulatedStackHelper"))
				.map(f -> {
					final String c = f.getClassName();
					return c.substring(c.lastIndexOf('.') + 1) + "." + f.getMethodName();
				}).findFirst().orElse("?"));
	}

	// ---------------------------------------------------------------------------------------------
	// subscriber order (bus.jsonl)

	private final Map<String, List<String>> subscribers = new java.util.LinkedHashMap<>();
	private Object subscribersEvent;
	private List<String> subscribersCollecting;

	/**
	 * One pre-interception = one subscriber about to handle {@code e} (the bus runs the composite
	 * pre-interceptor once per subscriber, in subscription order). For the first event of each class, the
	 * subscribers are recorded in that order.
	 */
	void subscriber(final org.palladiosimulator.analyzer.slingshot.eventdriver.entity.interceptors.InterceptorInformation info,
			final DESEvent e) {
		if (e != subscribersEvent) {
			subscribersEvent = e;
			final String k = e.getClass().getSimpleName();
			subscribersCollecting = subscribers.containsKey(k) ? null : new java.util.ArrayList<>();
			if (subscribersCollecting != null) {
				subscribers.put(k, subscribersCollecting);
			}
		}
		if (subscribersCollecting != null) {
			String n = info.getEnclosingType().map(Class::getSimpleName).orElse("?") + "." + info.getName();
			if (info instanceof final org.palladiosimulator.analyzer.slingshot.eventdriver.entity.Subscriber<?> sub
					&& sub.getReifiedClasses() != null && !sub.getReifiedClasses().isEmpty()) {
				n += "<" + sub.getReifiedClasses().stream().map(Class::getSimpleName)
						.collect(java.util.stream.Collectors.joining(",")) + ">";
			}
			subscribersCollecting.add(n);
		}
	}

	private void writeSubscribers() throws IOException {
		try (BufferedWriter w = writer("bus.jsonl")) {
			for (final Map.Entry<String, List<String>> en : subscribers.entrySet()) {
				final StringBuilder sb = new StringBuilder("[");
				for (final String n : en.getValue()) {
					if (sb.length() > 1) {
						sb.append(',');
					}
					Json.quote(sb, n);
				}
				w.write(new Json().str("ev", en.getKey()).raw("subs", sb.append(']').toString()).end());
				w.write('\n');
			}
		}
	}
}
