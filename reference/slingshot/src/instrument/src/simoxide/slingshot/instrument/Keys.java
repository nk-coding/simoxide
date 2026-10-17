package simoxide.slingshot.instrument;

import java.util.List;

import org.eclipse.emf.ecore.EAttribute;
import org.eclipse.emf.ecore.EObject;
import org.eclipse.emf.ecore.EReference;
import org.eclipse.emf.ecore.EStructuralFeature;
import org.palladiosimulator.edp2.models.measuringpoint.MeasuringPoint;
import org.palladiosimulator.metricspec.MetricDescription;
import org.palladiosimulator.metricspec.MetricSetDescription;

/**
 * Stable keys in SimOxide's conventions: measuring point keys as in measurements.csv
 * (docs/guide/formats.md §4, same rule as refsim's {@code Measurements.mpKey}) and PCM element ids.
 */
public final class Keys {
	private Keys() {
	}

	/** Id of the metric "Point in Time" in commonMetrics.metricspec. */
	public static final String POINT_IN_TIME_ID = "_NCRBos7pEeOX_4BzImuHbA";

	/** Index of the point-in-time component of a tuple metric, or -1. */
	public static int timeIndex(final MetricDescription md) {
		if (md instanceof final MetricSetDescription set) {
			final List<MetricDescription> sub = set.getSubsumedMetrics();
			for (int i = 0; i < sub.size(); i++) {
				if (POINT_IN_TIME_ID.equals(sub.get(i).getId())) {
					return i;
				}
			}
		}
		return -1;
	}

	/**
	 * EClass name followed by the ids of the non-containment single references and the values of
	 * the attributes, in metamodel feature order, e.g. {@code UsageScenarioMeasuringPoint[_id]}.
	 */
	public static String mpKey(final MeasuringPoint mp) {
		if (mp instanceof final org.palladiosimulator.edp2.models.measuringpoint.ResourceURIMeasuringPoint r) {
			final String frag = r.getResourceURI() == null ? "null"
					: org.eclipse.emf.common.util.URI.createURI(r.getResourceURI()).fragment();
			return "ResourceURIMeasuringPoint[" + frag + "|" + r.getMeasuringPoint() + "]";
		}
		final StringBuilder sb = new StringBuilder(mp.eClass().getName()).append('[');
		boolean first = true;
		for (final EStructuralFeature f : mp.eClass().getEAllStructuralFeatures()) {
			if (f.isDerived() || f.isTransient() || f.isMany()) {
				continue;
			}
			if (f.getEContainingClass().getEPackage() != mp.eClass().getEPackage()) {
				continue; // generic MeasuringPoint features (resourceURIRepresentation, repository)
			}
			final Object v = mp.eGet(f);
			final String s;
			if (f instanceof final EReference r) {
				if (r.isContainer() || r.isContainment()) {
					continue;
				}
				s = v == null ? "null" : id((EObject) v);
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

	/**
	 * Ids present in the input models (set by the runner after loading). Ids of elements created during
	 * the run (SPD adjustments: {@code EcoreUtil.generateUUID()}, time based, different in every run) are
	 * replaced by a structural alias, see {@link #alias(EObject)}. Null: no aliasing.
	 */
	public static volatile java.util.Set<String> inputIds;

	/** Collects the {@code id} attribute values of every object in the resource set. */
	public static java.util.Set<String> collectIds(final org.eclipse.emf.ecore.resource.ResourceSet rs) {
		final java.util.Set<String> ids = new java.util.HashSet<>();
		for (final org.eclipse.emf.ecore.resource.Resource r : new java.util.ArrayList<>(rs.getResources())) {
			for (final java.util.Iterator<EObject> it = r.getAllContents(); it.hasNext();) {
				final String id = rawId(it.next());
				if (id != null) {
					ids.add(id);
				}
			}
		}
		return ids;
	}

	private static String rawId(final EObject o) {
		final EStructuralFeature idf = o.eClass().getEStructuralFeature("id");
		if (idf != null && !idf.isMany()) {
			final Object id = o.eGet(idf);
			if (id != null) {
				return id.toString();
			}
		}
		return null;
	}

	/**
	 * PCM {@code id} attribute (or its alias for run-time created elements), else the EMF URI fragment.
	 * Never uses {@code toString()} or {@code hashCode()} of model objects (identity hash).
	 */
	public static String id(final EObject o) {
		if (o == null) {
			return null;
		}
		final String id = rawId(o);
		if (id != null) {
			final java.util.Set<String> known = inputIds;
			return known == null || known.contains(id) ? id : alias(o);
		}
		return org.eclipse.emf.ecore.util.EcoreUtil.getURI(o).fragment();
	}

	/**
	 * Deterministic key of an element created during the run: {@code ~EClass:entityName} for named
	 * elements (Slingshot's transformations name copies deterministically, e.g. {@code AppServer_1}),
	 * otherwise the alias/id of the container plus {@code /feature.index}.
	 */
	public static String alias(final EObject o) {
		final String n = name(o);
		if (n != null) {
			return "~" + o.eClass().getName() + ":" + n;
		}
		final EObject c = o.eContainer();
		if (c == null) {
			return "~" + o.eClass().getName();
		}
		final EReference f = o.eContainmentFeature();
		String seg = f.getName();
		if (f.isMany()) {
			seg += "." + ((List<?>) c.eGet(f)).indexOf(o);
		}
		final String pid = id(c);
		return (pid.startsWith("~") ? pid : "~" + pid) + "/" + seg;
	}

	/** {@code entityName} if the object has one, else null. */
	public static String name(final EObject o) {
		if (o == null) {
			return null;
		}
		final EStructuralFeature nf = o.eClass().getEStructuralFeature("entityName");
		if (nf != null) {
			final Object n = o.eGet(nf);
			return n == null ? null : n.toString();
		}
		return null;
	}

	public static String csvField(final String s) {
		if (s.indexOf(',') >= 0 || s.indexOf('"') >= 0 || s.indexOf('\n') >= 0) {
			return '"' + s.replace("\"", "\"\"") + '"';
		}
		return s;
	}
}
