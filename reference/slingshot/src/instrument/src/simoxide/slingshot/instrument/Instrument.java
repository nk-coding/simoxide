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
		final long i = "math".equals(s) ? tapeMath++ : tapeStoex++;
		try {
			tape.write(new Json().str("k", "u").str("s", s).integer("i", i).num("u", u).integer("e", eventIndex)
					.end());
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
}
