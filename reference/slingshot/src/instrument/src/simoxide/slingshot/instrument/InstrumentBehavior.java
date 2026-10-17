package simoxide.slingshot.instrument;

import org.palladiosimulator.analyzer.slingshot.common.events.DESEvent;
import org.palladiosimulator.analyzer.slingshot.core.extension.SimulationBehaviorExtension;
import org.palladiosimulator.analyzer.slingshot.eventdriver.annotations.PostIntercept;
import org.palladiosimulator.analyzer.slingshot.eventdriver.annotations.PreIntercept;
import org.palladiosimulator.analyzer.slingshot.eventdriver.annotations.Subscribe;
import org.palladiosimulator.analyzer.slingshot.eventdriver.annotations.eventcontract.OnEvent;
import org.palladiosimulator.analyzer.slingshot.eventdriver.entity.interceptors.InterceptorInformation;
import org.palladiosimulator.analyzer.slingshot.eventdriver.returntypes.InterceptionResult;
import org.palladiosimulator.analyzer.slingshot.eventdriver.returntypes.Result;

import jakarta.inject.Inject;

/**
 * Observes every dispatched event. Registered only while {@link Instrument} is active (isActive), so a
 * run without instrumentation has no extra subscriber or interceptor. Never schedules events, never
 * fails a pre-interception (that would skip the handler), never throws (a throwing handler would be
 * disposed by RxJava).
 */
@OnEvent(when = DESEvent.class)
public class InstrumentBehavior implements SimulationBehaviorExtension {

	@Inject
	public InstrumentBehavior() {
	}

	@Override
	public boolean isActive() {
		return Instrument.active();
	}

	/** Present so that every event has at least one subscriber and therefore reaches {@link #pre}. */
	@Subscribe
	public void onEvent(final DESEvent event) {
	}

	@PreIntercept
	public InterceptionResult pre(final InterceptorInformation info, final DESEvent event) {
		final Instrument r = Instrument.current();
		if (r != null) {
			try {
				r.dispatched(event);
				if (r.spd != null) {
					r.spd.pre(info, event);
				}
			} catch (final Throwable t) {
				r.fail(t);
			}
		}
		return InterceptionResult.success();
	}

	@PostIntercept
	public InterceptionResult post(final InterceptorInformation info, final DESEvent event, final Result<?> result) {
		final Instrument r = Instrument.current();
		if (r != null && r.spd != null) {
			try {
				r.spd.post(info, event, result);
			} catch (final Throwable t) {
				r.fail(t);
			}
		}
		return InterceptionResult.success();
	}
}
