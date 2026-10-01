package refsim;

import java.util.Map;

import org.palladiosimulator.recorderframework.core.config.IRecorderConfiguration;
import org.palladiosimulator.recorderframework.core.config.IRecorderConfigurationFactory;

/** Recorder configuration factory for the in-memory refsim recorder (replaces EDP2). */
public class RefsimRecorderConfigurationFactory implements IRecorderConfigurationFactory {
    @Override
    public void initialize(Map<String, Object> configuration) {
    }

    @Override
    public IRecorderConfiguration createRecorderConfiguration(Map<String, Object> configuration) {
        RefsimRecorderConfiguration c = new RefsimRecorderConfiguration();
        c.setConfiguration(configuration);
        return c;
    }

    @Override
    public void finalizeRecorderConfigurationFactory() {
    }
}
