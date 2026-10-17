package simoxide.slingshot.runner;

import org.eclipse.equinox.app.IApplication;
import org.eclipse.equinox.app.IApplicationContext;

/** Equinox entry point: {@code -application simoxide.slingshot.runner.app}. */
public class RunnerApp implements IApplication {

	@Override
	public Object start(final IApplicationContext context) throws Exception {
		final String[] args = (String[]) context.getArguments().get(IApplicationContext.APPLICATION_ARGS);
		return Integer.valueOf(Runner.main(args));
	}

	@Override
	public void stop() {
	}
}
