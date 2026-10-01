package refsim;

import org.palladiosimulator.measurementframework.MeasuringValue;
import org.palladiosimulator.recorderframework.core.AbstractRecorder;
import org.palladiosimulator.recorderframework.core.config.IRecorderConfiguration;

/**
 * Recorder that forwards every measuring value to the current run's {@link Measurements} sink (in
 * emission order). One instance per calculator (= measuring point x metric).
 */
public class RefsimRecorder extends AbstractRecorder {
    private Measurements.Series series;

    @Override
    public void initialize(IRecorderConfiguration recorderConfiguration) {
        RefsimRecorderConfiguration c = (RefsimRecorderConfiguration) recorderConfiguration;
        series = Measurements.current().series(c.getMeasuringPoint(), c.getRecorderAcceptedMetric());
    }

    @Override
    public void writeData(MeasuringValue measurement) {
        series.add(measurement);
    }

    @Override
    public void flush() {
    }
}
