package refsim;

import java.io.IOException;
import java.io.Writer;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

import javax.measure.Measure;

import org.eclipse.emf.ecore.EAttribute;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EReference;
import org.eclipse.emf.ecore.EStructuralFeature;
import org.palladiosimulator.edp2.models.measuringpoint.MeasuringPoint;
import org.palladiosimulator.measurementframework.MeasuringValue;
import org.palladiosimulator.metricspec.MetricDescription;

import refsim.trace.Trace;

/**
 * Per-run measurement sink fed by {@link RefsimRecorder}. Series are keyed by
 * (measuring point key, metric name); rows keep emission order.
 */
public final class Measurements {

    private static Measurements current;

    public static Measurements current() {
        if (current == null) {
            throw new IllegalStateException("no refsim run active");
        }
        return current;
    }

    static void begin() {
        current = new Measurements();
    }

    static Measurements end() {
        Measurements m = current;
        current = null;
        return m;
    }

    public final class Series {
        final String mp;
        final String metric;
        final List<double[]> rows = new ArrayList<>();
        String units = "";

        Series(String mp, String metric) {
            this.mp = mp;
            this.metric = metric;
        }

        public void add(MeasuringValue v) {
            List<Measure<?, ?>> l = v.asList();
            // normalise to (point in time, value...): e.g. "Resource Demand Tuple" is (demand, time)
            int timeIdx = timeIndex(v.getMetricDesciption());
            double[] row = new double[l.size()];
            StringBuilder u = new StringBuilder();
            for (int i = 0, k = 1; i < row.length; i++) {
                Measure<?, ?> m = l.get(i);
                int pos = i == timeIdx ? 0 : (timeIdx < 0 ? i : k++);
                row[pos] = ((Number) m.getValue()).doubleValue();
                if (i > 0) {
                    u.append(';');
                }
                u.append(m.getUnit());
            }
            if (rows.isEmpty()) {
                units = u.toString();
            }
            rows.add(row);
            total++;
            if (Trace.ON) {
                Trace.measurement(mp, metric, row);
            }
        }
    }

    static final String POINT_IN_TIME_ID = "_NCRBos7pEeOX_4BzImuHbA";

    static int timeIndex(MetricDescription md) {
        if (md instanceof org.palladiosimulator.metricspec.MetricSetDescription) {
            List<?> sub = ((org.palladiosimulator.metricspec.MetricSetDescription) md).getSubsumedMetrics();
            for (int i = 0; i < sub.size(); i++) {
                if (POINT_IN_TIME_ID.equals(((MetricDescription) sub.get(i)).getId())) {
                    return i;
                }
            }
        }
        return -1;
    }

    private final Map<String, Series> series = new LinkedHashMap<>();
    long total;

    public Series series(MeasuringPoint mp, MetricDescription metric) {
        String k = mpKey(mp);
        String name = metric.getName();
        return series.computeIfAbsent(k + "\u0000" + name, x -> new Series(k, name));
    }

    /**
     * Stable key of a measuring point: EClass name followed by the ids of its (non-containment)
     * references and the values of its own attributes, in metamodel feature order, e.g.
     * {@code UsageScenarioMeasuringPoint[_LPnI8CHdEd6lJo4DCALHMw]} or
     * {@code ActiveResourceMeasuringPoint[_procResId|replicaID=0]}.
     */
    public static String mpKey(MeasuringPoint mp) {
        if (mp instanceof org.palladiosimulator.edp2.models.measuringpoint.ResourceURIMeasuringPoint) {
            // created by SimuLizar/SimuCom for passive resources: resourceURI is an absolute file URI ->
            // keep only its fragment (element id) plus the embedded string representation
            var r = (org.palladiosimulator.edp2.models.measuringpoint.ResourceURIMeasuringPoint) mp;
            String frag = r.getResourceURI() == null ? "null"
                    : org.eclipse.emf.common.util.URI.createURI(r.getResourceURI()).fragment();
            return "ResourceURIMeasuringPoint[" + frag + "|" + r.getMeasuringPoint() + "]";
        }
        StringBuilder sb = new StringBuilder(mp.eClass().getName()).append('[');
        boolean first = true;
        for (EStructuralFeature f : mp.eClass().getEAllStructuralFeatures()) {
            if (f.isDerived() || f.isTransient() || f.isMany()) {
                continue;
            }
            if (f.getEContainingClass().getEPackage() != mp.eClass().getEPackage()) {
                continue; // generic MeasuringPoint features (resourceURIRepresentation, repository)
            }
            Object v = mp.eGet(f);
            String s;
            if (f instanceof EReference) {
                EReference r = (EReference) f;
                if (r.isContainer() || r.isContainment()) {
                    continue;
                }
                s = v == null ? "null" : idOf((EObject) v);
            } else if (f instanceof EAttribute) {
                s = f.getName() + "=" + v;
            } else {
                continue;
            }
            if (!first) {
                sb.append('|');
            }
            first = false;
            sb.append(s);
        }
        return sb.append(']').toString();
    }

    static String idOf(EObject o) {
        EStructuralFeature idf = o.eClass().getEStructuralFeature("id");
        if (idf != null) {
            Object id = o.eGet(idf);
            if (id != null) {
                return id.toString();
            }
        }
        return org.eclipse.emf.ecore.util.EcoreUtil.getURI(o).fragment();
    }

    public static String csvField(String s) {
        if (s.indexOf(',') >= 0 || s.indexOf('"') >= 0 || s.indexOf('\n') >= 0) {
            return '"' + s.replace("\"", "\"\"") + '"';
        }
        return s;
    }

    /** CSV: measuring_point,metric,time,value (series sorted by key, rows in emission order). */
    public void writeCsv(Writer w) throws IOException {
        w.write("measuring_point,metric,time,value\n");
        TreeMap<String, Series> sorted = new TreeMap<>();
        for (Series s : series.values()) {
            sorted.put(s.mp + "\u0000" + s.metric, s);
        }
        StringBuilder sb = new StringBuilder();
        for (Series s : sorted.values()) {
            String prefix = csvField(s.mp) + "," + csvField(s.metric) + ",";
            for (double[] row : s.rows) {
                sb.setLength(0);
                sb.append(prefix);
                for (int i = 0; i < row.length; i++) {
                    if (i > 0) {
                        sb.append(',');
                    }
                    sb.append(Trace.num(row[i]));
                }
                sb.append('\n');
                w.write(sb.toString());
            }
        }
    }

    public String summary() {
        StringBuilder sb = new StringBuilder();
        TreeMap<String, Series> sorted = new TreeMap<>();
        for (Series s : series.values()) {
            sorted.put(s.mp + "\u0000" + s.metric, s);
        }
        for (Series s : sorted.values()) {
            double sum = 0;
            for (double[] r : s.rows) {
                sum += r[r.length - 1];
            }
            sb.append(String.format("  %-70s %-40s n=%-6d mean=%s units=%s%n", s.mp, s.metric, s.rows.size(),
                    s.rows.isEmpty() ? "-" : Trace.num(sum / s.rows.size()), s.units));
        }
        return sb.toString();
    }
}
