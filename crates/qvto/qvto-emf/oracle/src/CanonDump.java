import java.util.*;

import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.emf.ecore.resource.*;
import org.eclipse.emf.ecore.util.EcoreUtil;

/**
 * Canonical dump of resources, the strict model comparison of qvto-rs (same format as
 * crates/qvto-emf/src/canon.rs). One JSON line per resource and per object, objects in
 * containment-tree order:
 *
 *   {"resource":"<label>","roots":N}
 *   {"path":"/0/@f.1","type":"<nsURI>#<Class>","attrs":{...},"refs":{...},"set":[...]}
 *
 * attrs: every non-transient, non-derived attribute with its value (EcoreUtil.convertToString;
 *   many-valued: a list), defaults included. refs: every non-transient, non-derived,
 *   non-containment, non-container reference that is not empty: targets as
 *   "<label>#<path>", "!<path>" (object in no resource), "?<last segment>#<fragment>" (proxy).
 * set: the unsettable features that are set. Paths always use "@feature.index" for
 * many-valued features and the root index. Labels: the decoded last segment of file URIs,
 * otherwise the URI.
 */
public final class CanonDump {
    private CanonDump() {}

    public static String dump(List<Resource> resources) {
        StringBuilder sb = new StringBuilder();
        for (Resource r : resources) {
            sb.append("{\"resource\":");
            str(sb, label(r));
            sb.append(",\"roots\":").append(r.getContents().size()).append("}\n");
            for (EObject root : r.getContents()) {
                obj(sb, root);
                for (TreeIterator<EObject> it = root.eAllContents(); it.hasNext(); ) obj(sb, it.next());
            }
        }
        return sb.toString();
    }

    static String label(Resource r) {
        URI u = r.getURI();
        if (u.isFile() && u.lastSegment() != null) return URI.decode(u.lastSegment());
        return u.toString();
    }

    static String path(EObject o) {
        List<String> segs = new ArrayList<>();
        EObject cur = o;
        while (cur.eContainer() != null && ((InternalEObject) cur).eDirectResource() == null) {
            EReference f = cur.eContainmentFeature();
            EObject p = cur.eContainer();
            if (f.isMany()) segs.add("@" + f.getName() + "." + ((List<?>) p.eGet(f, false)).indexOf(cur));
            else segs.add("@" + f.getName());
            cur = p;
        }
        Resource r = ((InternalEObject) cur).eDirectResource();
        StringBuilder sb = new StringBuilder("/");
        sb.append(r == null ? "" : String.valueOf(r.getContents().indexOf(cur)));
        for (int i = segs.size() - 1; i >= 0; i--) sb.append('/').append(segs.get(i));
        return sb.toString();
    }

    static String target(EObject t) {
        if (t.eIsProxy()) {
            URI u = ((InternalEObject) t).eProxyURI();
            String last = u.trimFragment().lastSegment();
            if (last == null) last = u.trimFragment().toString();
            return "?" + URI.decode(last) + "#" + u.fragment();
        }
        Resource r = t.eResource();
        if (r == null) return "!" + path(t);
        return label(r) + "#" + path(t);
    }

    static String typeName(EClass c) {
        EPackage p = c.getEPackage();
        String ns = p == null ? null : p.getNsURI();
        return (ns == null ? (p == null ? "" : p.getName()) : ns) + "#" + c.getName();
    }

    static void obj(StringBuilder sb, EObject o) {
        EClass c = o.eClass();
        sb.append("{\"path\":");
        str(sb, path(o));
        sb.append(",\"type\":");
        str(sb, typeName(c));
        sb.append(",\"attrs\":{");
        boolean first = true;
        List<String> set = new ArrayList<>();
        for (EStructuralFeature f : c.getEAllStructuralFeatures()) {
            if (f.isTransient() || f.isDerived()) continue;
            if (f.isUnsettable() && o.eIsSet(f)) set.add(f.getName());
            if (!(f instanceof EAttribute)) continue;
            EDataType dt = ((EAttribute) f).getEAttributeType();
            if (!first) sb.append(',');
            first = false;
            str(sb, f.getName());
            sb.append(':');
            Object v = o.eGet(f);
            if (f.isMany()) {
                sb.append('[');
                boolean ff = true;
                for (Object x : (List<?>) v) {
                    if (!ff) sb.append(',');
                    ff = false;
                    value(sb, dt, x);
                }
                sb.append(']');
            } else value(sb, dt, v);
        }
        sb.append("},\"refs\":{");
        first = true;
        for (EStructuralFeature f : c.getEAllStructuralFeatures()) {
            if (!(f instanceof EReference)) continue;
            EReference r = (EReference) f;
            if (r.isContainment() || r.isContainer() || r.isTransient() || r.isDerived()) continue;
            Object v = o.eGet(r, false);
            if (r.isMany()) {
                List<?> l = (List<?>) v;
                if (l.isEmpty()) continue;
                if (!first) sb.append(',');
                first = false;
                str(sb, r.getName());
                sb.append(":[");
                boolean ff = true;
                for (Object x : l) {
                    if (!ff) sb.append(',');
                    ff = false;
                    str(sb, target((EObject) x));
                }
                sb.append(']');
            } else {
                if (v == null) continue;
                if (!first) sb.append(',');
                first = false;
                str(sb, r.getName());
                sb.append(':');
                str(sb, target((EObject) v));
            }
        }
        sb.append('}');
        if (!set.isEmpty()) {
            sb.append(",\"set\":[");
            for (int i = 0; i < set.size(); i++) {
                if (i > 0) sb.append(',');
                str(sb, set.get(i));
            }
            sb.append(']');
        }
        sb.append("}\n");
    }

    static void value(StringBuilder sb, EDataType dt, Object v) {
        if (v == null) {
            sb.append("null");
            return;
        }
        String s;
        try {
            s = EcoreUtil.convertToString(dt, v);
        } catch (RuntimeException e) {
            s = String.valueOf(v);
        }
        if (s == null) sb.append("null");
        else str(sb, s);
    }

    static void str(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char ch = s.charAt(i);
            switch (ch) {
                case '"': sb.append("\\\""); break;
                case '\\': sb.append("\\\\"); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                default:
                    if (ch < 0x20) sb.append(String.format("\\u%04x", (int) ch));
                    else sb.append(ch);
            }
        }
        sb.append('"');
    }
}
