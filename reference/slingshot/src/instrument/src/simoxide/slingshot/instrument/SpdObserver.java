package simoxide.slingshot.instrument;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.List;

import org.apache.log4j.AppenderSkeleton;
import org.apache.log4j.Level;
import org.apache.log4j.Logger;
import org.apache.log4j.spi.LoggingEvent;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.resource.Resource;
import org.palladiosimulator.analyzer.slingshot.behavior.spd.data.ModelAdjustmentRequested;
import org.palladiosimulator.analyzer.slingshot.common.events.DESEvent;
import org.palladiosimulator.analyzer.slingshot.common.events.modelchanges.ModelAdjusted;
import org.palladiosimulator.analyzer.slingshot.eventdriver.entity.interceptors.InterceptorInformation;
import org.palladiosimulator.analyzer.slingshot.eventdriver.returntypes.Result;
import org.palladiosimulator.pcm.allocation.Allocation;
import org.palladiosimulator.pcm.allocation.AllocationContext;
import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.resourceenvironment.LinkingResource;
import org.palladiosimulator.pcm.resourceenvironment.ResourceContainer;
import org.palladiosimulator.pcm.resourceenvironment.ResourceEnvironment;
import org.palladiosimulator.spd.ScalingPolicy;
import org.palladiosimulator.semanticspd.Configuration;
import org.palladiosimulator.semanticspd.ElasticInfrastructureCfg;
import org.palladiosimulator.semanticspd.ServiceGroupCfg;
import org.palladiosimulator.semanticspd.TargetGroupCfg;

/**
 * SPD decision log ({@code spd.jsonl}), built without changing Slingshot's code:
 * <ul>
 * <li>Every policy's handler is a {@code Subscriber} whose handler is the inner class
 * {@code SPDAdjustorContext$PublishResultingEventFilter}. On the first pre-interception of such a
 * subscriber, the context's {@code FilterChain.filters} entries are replaced in place
 * ({@code List.set}, no structural change) by proxies of the {@code Filter} interface that delegate to
 * the original filter and record its result. A {@code policy} record describes the chain.</li>
 * <li>Each handler invocation gives one {@code eval} record: event, policy, the filters that ran with
 * their result ({@code ok} / {@code disregard} + reason / {@code exception}) and whether a
 * {@code ModelAdjustmentRequested} came out.</li>
 * <li>Around {@code SpdAdjustmentBehavior.onModelAdjustmentRequested}: an {@code adjust} record with a
 * model summary before and after the QVTo run and the {@code ModelAdjusted} result.</li>
 * <li>log4j messages of the SPD adjustment packages (QVTo log, "RECONFIGURATION WAS ...") as {@code log}
 * records.</li>
 * </ul>
 */
final class SpdObserver {

	private static final String PUBLISHER = "SPDAdjustorContext$PublishResultingEventFilter";
	private static final String ADJUSTMENT = "SpdAdjustmentBehavior";
	private static final String LOG_ROOT = "org.palladiosimulator.analyzer.slingshot.behavior.spd.adjustment";

	private final Instrument run;
	private final BufferedWriter out;
	private final boolean all;
	/** Contexts seen so far: (publisher handler object, record), compared with ==, never hashed. */
	private final List<Object[]> contexts = new ArrayList<>();
	private Eval eval;
	private String before;
	private Object adjustmentSubscriber;
	long evaluations, requests;
	private final Capture capture = new Capture();
	private final Logger logRoot = Logger.getLogger(LOG_ROOT);
	private final Level oldLevel;
	private final boolean oldAdditivity;

	SpdObserver(final Instrument run, final BufferedWriter out) {
		this.run = run;
		this.out = out;
		this.all = "all".equals(run.opt.spdLevel);
		oldLevel = logRoot.getLevel();
		oldAdditivity = logRoot.getAdditivity();
		logRoot.setLevel(Level.DEBUG);
		logRoot.setAdditivity(false);
		logRoot.addAppender(capture);
	}

	void close() throws IOException {
		logRoot.removeAppender(capture);
		logRoot.setLevel(oldLevel);
		logRoot.setAdditivity(oldAdditivity);
		out.close();
	}

	private void write(final String line) {
		try {
			out.write(line);
			out.write('\n');
		} catch (final IOException e) {
			throw new UncheckedIOException(e);
		}
	}

	private static final class Eval {
		final Object subscriber;
		final String policy;
		final DESEvent event;
		final StringBuilder filters = new StringBuilder("[");
		boolean targetGroupRejected;

		Eval(final Object subscriber, final String policy, final DESEvent event) {
			this.subscriber = subscriber;
			this.policy = policy;
			this.event = event;
		}
	}

	// ---------------------------------------------------------------------------------------------

	void pre(final InterceptorInformation info, final DESEvent e) throws Exception {
		final String enclosing = info.getEnclosingType().map(Class::getName).orElse("");
		final Object handler = handlerOf(info);
		if (handler != null && handler.getClass().getName().endsWith(PUBLISHER)) {
			final Object[] ctx = context(handler);
			eval = new Eval(info, (String) ctx[1], e);
		} else if (enclosing.contains(ADJUSTMENT) && e instanceof final ModelAdjustmentRequested m) {
			before = summary(m.getScalingPolicy());
			adjustmentSubscriber = info;
		}
	}

	void post(final InterceptorInformation info, final DESEvent e, final Result<?> result) throws Exception {
		if (eval != null && eval.subscriber == info) {
			final Eval ev = eval;
			eval = null;
			evaluations++;
			ModelAdjustmentRequested req = null;
			for (final Object o : result.getResultEvents()) {
				if (o instanceof final ModelAdjustmentRequested m) {
					req = m;
				}
			}
			if (req != null) {
				requests++;
			}
			if (!all && req == null && ev.targetGroupRejected) {
				return;
			}
			write(new Json().str("k", "eval").num("t", ev.event.time()).integer("e", run.eventIndex)
					.str("ev", ev.event.getName()).str("policy", ev.policy)
					.raw("filters", ev.filters.append(']').toString()).bool("request", req != null).end());
			return;
		}
		if (before != null && info == adjustmentSubscriber && e instanceof final ModelAdjustmentRequested m) {
			boolean ok = false;
			int changes = 0;
			for (final Object o : result.getResultEvents()) {
				if (o instanceof final ModelAdjusted a) {
					ok = a.isWasSuccessful();
					changes = a.getChanges().size();
				}
			}
			write(new Json().str("k", "adjust").num("t", e.time()).integer("e", run.eventIndex)
					.str("policy", m.getScalingPolicy().getId())
					.str("policy_name", m.getScalingPolicy().getEntityName()).bool("ok", ok)
					.integer("changes", changes).raw("before", before).raw("after", summary(m.getScalingPolicy()))
					.end());
			before = null;
			adjustmentSubscriber = null;
		}
	}

	// ---------------------------------------------------------------------------------------------
	// filter chain wrapping (reflection; SPD's interpreter packages are not exported)

	private static Object handlerOf(final InterceptorInformation info) {
		try {
			final Field f = field(info.getClass(), "handler");
			return f == null ? null : f.get(info);
		} catch (final ReflectiveOperationException ex) {
			return null;
		}
	}

	private static Field field(Class<?> c, final String name) {
		for (; c != null; c = c.getSuperclass()) {
			try {
				final Field f = c.getDeclaredField(name);
				f.setAccessible(true);
				return f;
			} catch (final NoSuchFieldException ex) {
				// next
			}
		}
		return null;
	}

	/** Returns {handler, policyId}; wraps the chain the first time. */
	private Object[] context(final Object handler) throws Exception {
		for (final Object[] c : contexts) {
			if (c[0] == handler) {
				return c;
			}
		}
		final Object ctx = field(handler.getClass(), "this$0").get(handler);
		final ScalingPolicy policy = (ScalingPolicy) ctx.getClass().getMethod("getScalingPolicy").invoke(ctx);
		final Object chain = ctx.getClass().getMethod("getFilterChain").invoke(ctx);
		@SuppressWarnings("unchecked")
		final List<Object> filters = (List<Object>) field(chain.getClass(), "filters").get(chain);
		final StringBuilder names = new StringBuilder("[");
		for (int i = 0; i < filters.size(); i++) {
			final Object f = filters.get(i);
			final String name = f.getClass().getSimpleName();
			if (i > 0) {
				names.append(',');
			}
			Json.quote(names, name);
			final Class<?> iface = filterInterface(f.getClass());
			if (iface != null) {
				filters.set(i, Proxy.newProxyInstance(iface.getClassLoader(), new Class<?>[] { iface },
						new Recorder(f, name)));
			}
		}
		names.append(']');
		final Object[] rec = { handler, policy.getId() };
		contexts.add(rec);
		write(new Json().str("k", "policy").str("id", policy.getId()).str("name", policy.getEntityName())
				.bool("active", policy.isActive())
				.str("target_group", policy.getTargetGroup() == null ? null : policy.getTargetGroup().getId())
				.str("adjustment", policy.getAdjustmentType() == null ? null
						: policy.getAdjustmentType().eClass().getName())
				.str("trigger", policy.getScalingTrigger() == null ? null
						: policy.getScalingTrigger().eClass().getName())
				.raw("chain", names.toString()).end());
		return rec;
	}

	private static Class<?> filterInterface(Class<?> c) {
		for (; c != null; c = c.getSuperclass()) {
			for (final Class<?> i : c.getInterfaces()) {
				if (i.getName().endsWith(".entities.Filter")) {
					return i;
				}
			}
		}
		return null;
	}

	/** Filter proxy: delegates, then appends [name, result, reason] to the current evaluation. */
	private final class Recorder implements InvocationHandler {
		private final Object target;
		private final String name;

		Recorder(final Object target, final String name) {
			this.target = target;
			this.name = name;
		}

		@Override
		public Object invoke(final Object proxy, final Method m, final Object[] args) throws Throwable {
			if (!"doProcess".equals(m.getName())) {
				if ("equals".equals(m.getName())) {
					return proxy == args[0];
				}
				if ("hashCode".equals(m.getName())) {
					return 0; // never used by Slingshot; avoids identity hashes
				}
				if ("toString".equals(m.getName())) {
					return "Recorder[" + name + "]";
				}
				return m.invoke(target, args);
			}
			final Object r;
			try {
				r = m.invoke(target, args);
			} catch (final InvocationTargetException ex) {
				record("exception", ex.getCause().getClass().getName() + ": " + ex.getCause().getMessage());
				throw ex.getCause();
			}
			final String kind = r == null ? "null" : r.getClass().getSimpleName();
			if ("Disregard".equals(kind)) {
				final Object reason = r.getClass().getMethod("reason").invoke(r);
				record("disregard", reasonText(reason));
				if (name.equals("TargetGroupChecker") && eval != null) {
					eval.targetGroupRejected = true;
				}
			} else {
				record("Success".equals(kind) ? "ok" : kind, null);
			}
			return r;
		}

		private void record(final String result, final String reason) {
			final Eval ev = eval;
			if (ev == null) {
				return;
			}
			if (ev.filters.length() > 1) {
				ev.filters.append(',');
			}
			ev.filters.append('[');
			Json.quote(ev.filters, name);
			ev.filters.append(',');
			Json.quote(ev.filters, result);
			if (reason != null) {
				ev.filters.append(',');
				Json.quote(ev.filters, reason);
			}
			ev.filters.append(']');
		}
	}

	private static String reasonText(final Object reason) {
		if (reason == null) {
			return "";
		}
		if (reason instanceof final Throwable t) {
			return t.getClass().getName() + ": " + t.getMessage();
		}
		if (reason instanceof String || reason instanceof Number || reason instanceof Enum<?>) {
			return reason.toString();
		}
		return reason.getClass().getName();
	}

	// ---------------------------------------------------------------------------------------------
	// model summary

	/** Containers, allocations, links and semantic configuration, found via the policy's resource set. */
	static String summary(final ScalingPolicy policy) {
		final StringBuilder sb = new StringBuilder("{");
		final Resource r = policy.eResource();
		if (r == null || r.getResourceSet() == null) {
			return "{}";
		}
		ResourceEnvironment env = null;
		Allocation alloc = null;
		Configuration cfg = null;
		for (final Resource res : r.getResourceSet().getResources()) {
			for (final EObject o : res.getContents()) {
				if (o instanceof final ResourceEnvironment x && env == null) {
					env = x;
				} else if (o instanceof final Allocation x && alloc == null) {
					alloc = x;
				} else if (o instanceof final Configuration x && cfg == null) {
					cfg = x;
				}
			}
		}
		sb.append("\"containers\":[");
		if (env != null) {
			boolean first = true;
			for (final ResourceContainer c : env.getResourceContainer_ResourceEnvironment()) {
				sb.append(first ? "" : ",").append("[");
				Json.quote(sb, nz(c.getEntityName()));
				sb.append(',');
				Json.quote(sb, nz(Keys.id(c)));
				sb.append(']');
				first = false;
			}
		}
		sb.append("],\"links\":[");
		if (env != null) {
			boolean first = true;
			for (final LinkingResource l : env.getLinkingResources__ResourceEnvironment()) {
				sb.append(first ? "" : ",").append("[");
				Json.quote(sb, nz(l.getEntityName()));
				for (final ResourceContainer c : l.getConnectedResourceContainers_LinkingResource()) {
					sb.append(',');
					Json.quote(sb, nz(c.getEntityName()));
				}
				sb.append(']');
				first = false;
			}
		}
		sb.append("],\"allocation\":[");
		if (alloc != null) {
			boolean first = true;
			for (final AllocationContext ac : alloc.getAllocationContexts_Allocation()) {
				final AssemblyContext asm = ac.getAssemblyContext_AllocationContext();
				final ResourceContainer rc = ac.getResourceContainer_AllocationContext();
				sb.append(first ? "" : ",").append("[");
				Json.quote(sb, asm == null ? "" : nz(asm.getEntityName()));
				sb.append(',');
				Json.quote(sb, rc == null ? "" : nz(rc.getEntityName()));
				sb.append(']');
				first = false;
			}
		}
		sb.append("],\"target_groups\":[");
		if (cfg != null) {
			boolean first = true;
			for (final TargetGroupCfg t : cfg.getTargetCfgs()) {
				sb.append(first ? "" : ",").append("{\"type\":");
				Json.quote(sb, t.eClass().getName());
				sb.append(",\"elements\":[");
				final List<? extends EObject> els = t instanceof final ElasticInfrastructureCfg ei ? ei.getElements()
						: t instanceof final ServiceGroupCfg sg ? sg.getElements() : List.of();
				for (int i = 0; i < els.size(); i++) {
					if (i > 0) {
						sb.append(',');
					}
					final String n = Keys.name(els.get(i));
					Json.quote(sb, n == null ? nz(Keys.id(els.get(i))) : n);
				}
				sb.append("]}");
				first = false;
			}
		}
		return sb.append("]}").toString();
	}

	private static String nz(final String s) {
		return s == null ? "" : s;
	}

	// ---------------------------------------------------------------------------------------------

	private final class Capture extends AppenderSkeleton {
		@Override
		protected void append(final LoggingEvent ev) {
			final Object msg = ev.getMessage();
			final String text = msg instanceof String ? (String) msg : (msg == null ? "" : msg.getClass().getName());
			final String logger = ev.getLoggerName();
			write(new Json().str("k", "log").num("t", run.now).integer("e", run.eventIndex)
					.str("level", ev.getLevel().toString())
					.str("logger", logger.substring(logger.lastIndexOf('.') + 1)).str("msg", text).end());
		}

		@Override
		public void close() {
		}

		@Override
		public boolean requiresLayout() {
			return false;
		}
	}
}
