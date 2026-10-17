package simoxide.slingshot.instrument;

import org.palladiosimulator.analyzer.slingshot.core.extension.AbstractSlingshotExtension;

/** Slingshot extension (plugin.xml) that installs {@link InstrumentBehavior}. */
public class InstrumentModule extends AbstractSlingshotExtension {

	@Override
	protected void configure() {
		install(InstrumentBehavior.class);
	}

	@Override
	public String getName() {
		return InstrumentModule.class.getSimpleName();
	}
}
