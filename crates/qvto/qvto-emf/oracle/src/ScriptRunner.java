import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.emf.ecore.resource.*;
import org.eclipse.emf.ecore.resource.impl.*;
import org.eclipse.emf.ecore.util.EcoreUtil;

/**
 * Runs model-API scripts on dynamic EMF objects of the contract metamodel
 * (crates/qvto-model/src/contract.ecore) and prints each operation's result and the final state,
 * in the format of crates/qvto-emf/tests/common/script.rs.
 *
 * Script lines (values: s:<text> i:<int> l:<long> d:<double> b:<bool> e:<literal> o:<var> null):
 *   new v Class | set v f val | unset v f | add v f val [at] | remove v f val | removeat v f i
 *   setat v f i val | clear v f | move v f to from | setlist v f val... | res r uri
 *   root r v [at] | unroot r v | delete v
 */
public final class ScriptRunner {
    final EPackage pkg;
    final Map<String, EObject> objs = new LinkedHashMap<>();
    final Map<EObject, String> names = new HashMap<>();
    final Map<String, Resource> res = new LinkedHashMap<>();
    final Map<Resource, String> resNames = new HashMap<>();
    final ResourceSet rs = new ResourceSetImpl();
    final StringBuilder out = new StringBuilder();

    ScriptRunner(EPackage pkg) {
        this.pkg = pkg;
    }

    static void run(String ecore, Path in, Path outDir) throws IOException {
        ResourceSet mm = new ResourceSetImpl();
        // a fixed URI, so saves with schema locations do not depend on the checkout's path
        Resource r = mm.createResource(URI.createURI("http://qvto-rs/contract.ecore"));
        try (InputStream is = new FileInputStream(ecore)) {
            r.load(is, null);
        }
        EPackage pkg = (EPackage) r.getContents().get(0);
        Files.createDirectories(outDir);
        List<Path> scripts = new ArrayList<>();
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(in, "*.script")) {
            for (Path p : ds) scripts.add(p);
        }
        Collections.sort(scripts);
        StringBuilder results = new StringBuilder();
        for (Path p : scripts) {
            ScriptRunner sr = new ScriptRunner(pkg);
            for (String line : Files.readAllLines(p)) {
                if (line.isBlank()) continue;
                sr.out.append(line).append(" => ");
                try {
                    sr.out.append(sr.exec(line.trim().split(" ")));
                } catch (RuntimeException e) {
                    sr.out.append("err ").append(kind(e));
                }
                sr.out.append('\n');
            }
            sr.state();
            sr.saves();
            byte[] b = sr.out.toString().getBytes(StandardCharsets.UTF_8);
            String name = p.getFileName().toString().replace(".script", ".out");
            Files.write(outDir.resolve(name), b);
            results.append(p.getFileName()).append('\t').append(EmfOracle.fnv(b)).append('\n');
        }
        Files.write(outDir.resolve("results.tsv"), results.toString().getBytes(StandardCharsets.UTF_8));
    }

    static String kind(RuntimeException e) {
        String m = String.valueOf(e.getMessage());
        if (e instanceof IllegalStateException && (m.equals("NoSuchFeature") || m.equals("WrongMultiplicity"))) return m;
        if (e instanceof ClassCastException || e instanceof ArrayStoreException) return "TypeMismatch";
        if (e instanceof IndexOutOfBoundsException) return "IndexOutOfBounds";
        if (m.contains("no duplicates")) return "Duplicate";
        if (m.contains("no null")) return "NullNotAllowed";
        if (m.contains("Recursive containment")) return "RecursiveContainment";
        if (m.contains("not a valid classifier")) return "Abstract";
        return e.getClass().getSimpleName();
    }

    EStructuralFeature feature(EObject o, String name) {
        EStructuralFeature f = o.eClass().getEStructuralFeature(name);
        if (f == null) throw new IllegalStateException("NoSuchFeature");
        return f;
    }

    Object value(String s) {
        if (s.equals("null")) return null;
        String v = s.substring(2);
        switch (s.charAt(0)) {
            case 's': return v;
            case 'i': return Integer.valueOf(v);
            case 'l': return Long.valueOf(v);
            case 'd': return Double.valueOf(v);
            case 'b': return Boolean.valueOf(v);
            case 'e': return ((EEnum) pkg.getEClassifier("Kind")).getEEnumLiteral(v);
            case 'o': return objs.get(v);
            default: throw new IllegalArgumentException(s);
        }
    }

    @SuppressWarnings("unchecked")
    EList<Object> list(EObject o, EStructuralFeature f) {
        if (!f.isMany()) throw new IllegalStateException("WrongMultiplicity");
        return (EList<Object>) o.eGet(f);
    }

    String exec(String[] a) {
        switch (a[0]) {
            case "new": {
                EObject o = EcoreUtil.create((EClass) pkg.getEClassifier(a[2]));
                objs.put(a[1], o);
                names.put(o, a[1]);
                return "ok";
            }
            case "res": {
                Resource r = rs.createResource(URI.createURI(a[2]));
                res.put(a[1], r);
                resNames.put(r, a[1]);
                return "ok";
            }
            case "root": {
                Resource r = res.get(a[1]);
                EObject o = objs.get(a[2]);
                if (a.length > 3) {
                    r.getContents().add(Integer.parseInt(a[3]), o);
                    return "ok";
                }
                return "ok " + r.getContents().add(o);
            }
            case "unroot": return "ok " + res.get(a[1]).getContents().remove(objs.get(a[2]));
            case "delete": EcoreUtil.delete(objs.get(a[1])); return "ok";
        }
        EObject o = objs.get(a[1]);
        EStructuralFeature f = feature(o, a[2]);
        switch (a[0]) {
            case "set":
                if (f.isMany()) throw new IllegalStateException("WrongMultiplicity");
                o.eSet(f, value(a[3]));
                return "ok";
            case "unset": o.eUnset(f); return "ok";
            case "add":
                if (a.length > 4) {
                    list(o, f).add(Integer.parseInt(a[4]), value(a[3]));
                    return "ok";
                }
                return "ok " + list(o, f).add(value(a[3]));
            case "remove": return "ok " + list(o, f).remove(value(a[3]));
            case "removeat": list(o, f).remove(Integer.parseInt(a[3])); return "ok";
            case "setat": list(o, f).set(Integer.parseInt(a[3]), value(a[4])); return "ok";
            case "clear": list(o, f).clear(); return "ok";
            case "move": list(o, f).move(Integer.parseInt(a[3]), Integer.parseInt(a[4])); return "ok";
            case "setlist": {
                List<Object> vs = new ArrayList<>();
                for (int i = 3; i < a.length; i++) vs.add(value(a[i]));
                ECollections.setEList(list(o, f), vs);
                return "ok";
            }
            default: throw new IllegalArgumentException(a[0]);
        }
    }

    /** Each resource saved with default options and with the reference runner's options. */
    void saves() {
        for (Map.Entry<String, Resource> e : res.entrySet()) {
            for (int variant = 0; variant < 2; variant++) {
                out.append("~save ").append(e.getKey()).append(variant == 0 ? " default" : " runner").append('\n');
                Map<Object, Object> opts = new HashMap<>();
                if (variant == 1) {
                    opts.put(org.eclipse.emf.ecore.xmi.XMLResource.OPTION_ENCODING, "UTF-8");
                    opts.put(org.eclipse.emf.ecore.xmi.XMLResource.OPTION_SCHEMA_LOCATION, Boolean.TRUE);
                }
                try {
                    ByteArrayOutputStream b = new ByteArrayOutputStream();
                    e.getValue().save(b, opts);
                    out.append(b.toString(StandardCharsets.UTF_8));
                } catch (Exception x) {
                    Throwable c = x.getCause() != null ? x.getCause() : x;
                    out.append("err ").append(c.getClass().getSimpleName()).append('\n');
                }
            }
        }
    }

    String show(Object v) {
        if (v == null) return "null";
        if (v instanceof Enumerator) return ((Enumerator) v).getLiteral();
        if (v instanceof EObject) return names.getOrDefault(v, "?");
        return v.toString();
    }

    void state() {
        out.append("--\n");
        for (Map.Entry<String, Resource> e : res.entrySet()) {
            out.append("@").append(e.getKey()).append(" [");
            boolean first = true;
            for (EObject x : e.getValue().getContents()) {
                if (!first) out.append(',');
                first = false;
                out.append(show(x));
            }
            out.append("]\n");
        }
        for (Map.Entry<String, EObject> e : objs.entrySet()) {
            EObject o = e.getValue();
            InternalEObject io = (InternalEObject) o;
            out.append("#").append(e.getKey()).append(' ').append(o.eClass().getName());
            out.append(" c=").append(o.eContainer() == null ? "-" : show(o.eContainer()) + "." + o.eContainmentFeature().getName());
            out.append(" r=").append(o.eResource() == null ? "-" : resNames.get(o.eResource()));
            out.append(" dr=").append(io.eDirectResource() == null ? "-" : resNames.get(io.eDirectResource()));
            out.append('\n');
            for (EStructuralFeature f : o.eClass().getEAllStructuralFeatures()) {
                out.append("  ").append(f.getName()).append(o.eIsSet(f) ? "*" : "").append('=');
                Object v = o.eGet(f);
                if (f.isMany()) {
                    out.append('[');
                    boolean first = true;
                    for (Object x : (List<?>) v) {
                        if (!first) out.append(',');
                        first = false;
                        out.append(show(x));
                    }
                    out.append(']');
                } else out.append(show(v));
                out.append('\n');
            }
        }
    }
}
