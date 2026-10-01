import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.jar.*;

import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.emf.ecore.resource.*;
import org.eclipse.emf.ecore.resource.impl.*;
import org.eclipse.emf.ecore.util.EcoreUtil;
import org.eclipse.emf.ecore.xmi.impl.XMIResourceFactoryImpl;

/**
 * Golden oracle for crates/simoxide-model: loads PCM model directories with real EMF (standalone, the
 * generated 5.2.2 packages, default XMI load options, EcoreUtil.resolveAll) and writes the
 * canonical dump documented in crates/simoxide-model/src/canon.rs.
 *
 * Usage: PcmModelOracle <plugins dir> <list file: name TAB dir per line> <out dir>
 */
public class PcmModelOracle {
    static final String[] EXT = {"repository", "system", "resourceenvironment", "allocation", "usagemodel",
            "resourcetype", "monitorrepository", "measuringpoint"};

    public static void main(String[] args) throws Exception {
        File plugins = new File(args[0]);
        // register the generated packages
        EPackage[] pkgs = {
            org.palladiosimulator.pcm.PcmPackage.eINSTANCE,
            de.uka.ipd.sdq.identifier.IdentifierPackage.eINSTANCE,
            de.uka.ipd.sdq.units.UnitsPackage.eINSTANCE,
            de.uka.ipd.sdq.stoex.StoexPackage.eINSTANCE,
            de.uka.ipd.sdq.probfunction.ProbfunctionPackage.eINSTANCE,
            org.palladiosimulator.metricspec.MetricSpecPackage.eINSTANCE,
            org.palladiosimulator.edp2.models.measuringpoint.MeasuringpointPackage.eINSTANCE,
            org.palladiosimulator.monitorrepository.MonitorRepositoryPackage.eINSTANCE,
            org.palladiosimulator.monitorrepository.map.MapPackage.eINSTANCE,
            org.palladiosimulator.pcmmeasuringpoint.PcmmeasuringpointPackage.eINSTANCE,
            simulizarmeasuringpoint.SimulizarmeasuringpointPackage.eINSTANCE,
        };
        if (pkgs.length == 0) throw new IllegalStateException();
        Resource.Factory.Registry.INSTANCE.getExtensionToFactoryMap().put("*", new XMIResourceFactoryImpl());
        // pathmap + platform:/plugin mappings (as registered by the bundles' plugin.xml)
        for (File jar : plugins.listFiles((d, n) -> n.endsWith(".jar"))) {
            String bsn;
            try (JarFile jf = new JarFile(jar)) {
                if (jf.getManifest() == null) continue;
                bsn = jf.getManifest().getMainAttributes().getValue("Bundle-SymbolicName");
            }
            if (bsn == null) continue;
            bsn = bsn.split(";")[0].trim();
            URI base = URI.createURI("jar:" + jar.toURI() + "!/");
            URIConverter.URI_MAP.put(URI.createURI("platform:/plugin/" + bsn + "/"), base);
            if (bsn.equals("org.palladiosimulator.pcm.resources"))
                URIConverter.URI_MAP.put(URI.createURI("pathmap://PCM_MODELS/"), base.appendSegment("defaultModels").appendSegment(""));
            if (bsn.equals("org.palladiosimulator.metricspec.resources"))
                URIConverter.URI_MAP.put(URI.createURI("pathmap://METRIC_SPEC_MODELS/"), base);
        }
        Path out = Paths.get(args[2]);
        Files.createDirectories(out);
        for (String line : Files.readAllLines(Paths.get(args[1]))) {
            if (line.isBlank() || line.startsWith("#")) continue;
            String[] kv = line.split("\t");
            try {
                String dump = kv[1].equals("-") ? dumpBundled() : dumpDir(new File(kv[1]));
                Files.write(out.resolve(kv[0] + ".jsonl"), dump.getBytes(StandardCharsets.UTF_8));
            } catch (Throwable t) {
                System.out.println("FAILED " + kv[0] + ": " + t);
            }
        }
    }

    static Path baseDir;
    /** ID values literally present in each resource's XML (PCM's IdentifierImpl invents an ID for
     *  every Identifier without one; those are random and dumped as "<generated>"). */
    static Map<Resource, Set<String>> fileIds = new HashMap<>();

    static Set<String> idsOf(Resource r) {
        return fileIds.computeIfAbsent(r, k -> {
            Set<String> ids = new HashSet<>();
            try (InputStream in = r.getResourceSet().getURIConverter().createInputStream(r.getURI())) {
                javax.xml.parsers.SAXParserFactory f = javax.xml.parsers.SAXParserFactory.newInstance();
                f.setNamespaceAware(true);
                f.newSAXParser().parse(in, new org.xml.sax.helpers.DefaultHandler() {
                    @Override
                    public void startElement(String uri, String ln, String qn, org.xml.sax.Attributes a) {
                        for (int i = 0; i < a.getLength(); i++)
                            if (a.getQName(i).equals("id") || a.getQName(i).equals("xmi:id")) ids.add(a.getValue(i));
                    }
                });
            } catch (Exception e) {
                // unreadable: no ids
            }
            return ids;
        });
    }

    static boolean generatedId(EObject o) {
        String id = EcoreUtil.getID(o);
        return id != null && o.eResource() != null && !idsOf(o.eResource()).contains(id);
    }

    static final String[] BUNDLED = {"pathmap://PCM_MODELS/Palladio.resourcetype",
            "pathmap://PCM_MODELS/PrimitiveTypes.repository", "pathmap://PCM_MODELS/FailureTypes.repository",
            "pathmap://PCM_MODELS/Glassfish.repository", "pathmap://PCM_MODELS/default_event_middleware.repository",
            "pathmap://METRIC_SPEC_MODELS/commonMetrics.metricspec",
            "pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec"};

    /** The default models the crate bundles, loaded through their pathmap URIs. */
    static String dumpBundled() throws IOException {
        ResourceSet rs = newResourceSet();
        int errors = 0;
        for (String u : BUNDLED) {
            try {
                rs.getResource(URI.createURI(u), true);
            } catch (RuntimeException e) {
                errors++;
            }
        }
        EcoreUtil.resolveAll(rs);
        return dump(rs, errors, true);
    }

    static ResourceSet newResourceSet() {
        ResourceSet rs = new ResourceSetImpl();
        fileIds.clear();
        // unknown namespace URIs make EMF try to download them: fail fast instead (offline)
        rs.getURIConverter().getURIHandlers().add(0, new URIHandlerImpl() {
            @Override
            public boolean canHandle(URI uri) {
                return "http".equals(uri.scheme()) || "https".equals(uri.scheme());
            }

            @Override
            public InputStream createInputStream(URI uri, Map<?, ?> options) throws IOException {
                throw new IOException("offline: " + uri);
            }
        });
        return rs;
    }

    static String dumpDir(File dir) throws IOException {
        baseDir = dir.getAbsoluteFile().toPath().normalize();
        ResourceSet rs = newResourceSet();
        File[] files = dir.listFiles((d, n) -> {
            for (String e : EXT) if (n.endsWith("." + e)) return new File(d, n).isFile();
            return false;
        });
        Arrays.sort(files);
        int errors = 0;
        for (File f : files) {
            try {
                rs.getResource(URI.createFileURI(f.getAbsolutePath()), true);
            } catch (RuntimeException e) {
                errors++;
            }
        }
        try {
            EcoreUtil.resolveAll(rs);
        } catch (RuntimeException e) {
            errors++;
        }
        return dump(rs, errors, false);
    }

    /** Pathmap (bundled) resources are only dumped by the "_bundled" entry. */
    static String dump(ResourceSet rs, int errors, boolean bundled) {
        for (Resource r : rs.getResources()) errors += r.getErrors().size();
        List<Resource> res = new ArrayList<>();
        for (Resource r : rs.getResources())
            if (!r.getContents().isEmpty() && (bundled || !name(r).startsWith("pathmap:"))) res.add(r);
        res.sort(Comparator.comparing(PcmModelOracle::name));
        StringBuilder sb = new StringBuilder();
        sb.append("{\"emf_errors\":").append(errors).append("}\n");
        for (Resource r : res) {
            sb.append("{\"resource\":");
            str(sb, name(r));
            sb.append(",\"roots\":").append(r.getContents().size()).append("}\n");
            for (TreeIterator<EObject> it = EcoreUtil.getAllProperContents(r, false); it.hasNext(); ) {
                obj(sb, it.next());
                sb.append('\n');
            }
        }
        return sb.toString();
    }

    static String name(Resource r) {
        URI u = r.getURI();
        String s = u.toString();
        String pcm = "platform:/plugin/org.palladiosimulator.pcm.resources/defaultModels/";
        String met = "platform:/plugin/org.palladiosimulator.metricspec.resources/";
        if (s.startsWith(pcm)) return "pathmap://PCM_MODELS/" + s.substring(pcm.length());
        if (s.startsWith(met)) return "pathmap://METRIC_SPEC_MODELS/" + s.substring(met.length());
        if (u.isFile()) {
            Path p = Paths.get(u.toFileString()).toAbsolutePath().normalize();
            return baseDir.relativize(p).toString();
        }
        return s;
    }

    static String path(EObject o) {
        List<String> segs = new ArrayList<>();
        EObject cur = o;
        while (cur.eContainer() != null) {
            EReference f = cur.eContainmentFeature();
            if (f.isMany()) segs.add("@" + f.getName() + "." + ((List<?>) cur.eContainer().eGet(f, false)).indexOf(cur));
            else segs.add("@" + f.getName());
            cur = cur.eContainer();
        }
        int ri = cur.eResource().getContents().indexOf(cur);
        StringBuilder sb = new StringBuilder("/");
        if (ri != 0) sb.append(ri);
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
        return name(t.eResource()) + "#" + (generatedId(t) ? path(t) : t.eResource().getURIFragment(t));
    }

    static void obj(StringBuilder sb, EObject o) {
        EClass c = o.eClass();
        sb.append("{\"path\":");
        str(sb, path(o));
        sb.append(",\"id\":");
        String id = EcoreUtil.getID(o);
        if (id == null) sb.append("null");
        else if (generatedId(o)) str(sb, "<generated>");
        else str(sb, id);
        sb.append(",\"type\":");
        str(sb, c.getEPackage().getName() + ":" + c.getName());
        sb.append(",\"attrs\":{");
        boolean first = true;
        EAttribute ida = c.getEIDAttribute();
        for (EStructuralFeature f : c.getEAllStructuralFeatures()) {
            if (!(f instanceof EAttribute) || f.isTransient() || f.isDerived() || f == ida) continue;
            // computed by the measuring point implementation from the referenced element
            if (f.getEContainingClass().getName().equals("MeasuringPoint")
                    && (f.getName().equals("stringRepresentation") || f.getName().equals("resourceURIRepresentation")))
                continue;
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
                    value(sb, (EAttribute) f, x);
                }
                sb.append(']');
            } else value(sb, (EAttribute) f, v);
        }
        sb.append("},\"refs\":{");
        first = true;
        for (EStructuralFeature f : c.getEAllStructuralFeatures()) {
            if (!(f instanceof EReference)) continue;
            EReference r = (EReference) f;
            if (r.isContainment() || r.isContainer() || r.isTransient() || r.isDerived()) continue;
            Object v = o.eGet(r, true);
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
        sb.append("}}");
    }

    static void value(StringBuilder sb, EAttribute a, Object v) {
        if (v == null) sb.append("null");
        else if (v instanceof Double) str(sb, Double.toString((Double) v));
        else if (v instanceof Float) str(sb, Double.toString(((Float) v).doubleValue()));
        else if (v instanceof Integer || v instanceof Long || v instanceof Short || v instanceof Byte) sb.append(v);
        else if (v instanceof Boolean) sb.append(v);
        else if (v instanceof Enumerator) str(sb, ((Enumerator) v).getName());
        else if (v instanceof String) str(sb, (String) v);
        else str(sb, EcoreUtil.convertToString(a.getEAttributeType(), v));
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
