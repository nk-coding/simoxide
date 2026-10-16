import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.regex.*;

import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.emf.ecore.resource.*;
import org.eclipse.emf.ecore.resource.impl.*;
import org.eclipse.emf.ecore.util.EcoreUtil;
import org.eclipse.emf.ecore.xmi.XMLResource;
import org.eclipse.emf.ecore.xmi.impl.EcoreResourceFactoryImpl;
import org.eclipse.emf.ecore.xmi.impl.XMIResourceFactoryImpl;

/**
 * EMF oracle for qvto-emf (EMF 2.41 standalone, the reference runner's jars).
 *
 * Commands:
 *   models <config> <list> <outdir>
 *       Loads each model file of <list> (paths relative to the repository root) in its own
 *       resource set, saves it with the resource's default options to <outdir>/<n>.xmi, resolves
 *       all proxies and writes the canonical dump (see CanonDump) to <outdir>/<n>.jsonl, and
 *       writes <outdir>/results.tsv: path, status, error count, fnv64 of the save, of the dump.
 *   scripts <config> <contract.ecore> <indir> <outdir>
 *       Runs the model-API scripts of <indir> on dynamic EMF (see ScriptRunner).
 *   compare <config> <a> <b>
 *       Prints EQUAL or DIFFERENT: the Eclipse suite's TreeComparator on two model files.
 *
 * <config> lines (paths relative to the repository root, the current directory):
 *   map <from-uri-prefix> <path-prefix>   URIConverter.URI_MAP entry to a directory or file
 *   mmroot <dir>                          index the nsURIs of all .ecore files below <dir>
 *   alias <key> <nsURI>                   register an extra package-registry key
 */
public class EmfOracle {
    static Map<String, String> nsIndex = new LinkedHashMap<>();
    static Map<URI, URI> uriMap = new LinkedHashMap<>();
    static Map<String, String> aliases = new LinkedHashMap<>();
    static ResourceSet mmSet;

    public static void main(String[] args) throws Exception {
        Locale.setDefault(Locale.US);
        Resource.Factory.Registry.INSTANCE.getExtensionToFactoryMap().put("ecore", new EcoreResourceFactoryImpl());
        Resource.Factory.Registry.INSTANCE.getExtensionToFactoryMap().put("*", new XMIResourceFactoryImpl());
        EcorePackage.eINSTANCE.eClass();
        readConfig(Paths.get(args[1]));
        switch (args[0]) {
            case "models": models(Paths.get(args[2]), Paths.get(args[3])); break;
            case "compare": System.out.println(compare(args[2], args[3]) ? "EQUAL" : "DIFFERENT"); break;
            case "ecore": saveEcore(args[2]); break;
            case "scripts": ScriptRunner.run(args[2], Paths.get(args[3]), Paths.get(args[4])); break;
            default: throw new IllegalArgumentException(args[0]);
        }
    }

    static URI fileUri(String path) {
        return URI.createFileURI(new File(path).getAbsolutePath());
    }

    static void readConfig(Path conf) throws IOException {
        List<String> roots = new ArrayList<>();
        for (String line : Files.readAllLines(conf)) {
            line = line.trim();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] p = line.split("\\s+");
            switch (p[0]) {
                case "map": {
                    String to = new File(p[2]).getAbsolutePath();
                    if (p[1].endsWith("/")) to += "/";
                    URI target = URI.createFileURI(to);
                    URIConverter.URI_MAP.put(URI.createURI(p[1]), target);
                    break;
                }
                case "mmroot": roots.add(p[1]); break;
                case "alias": aliases.put(p[1], p[2]); break;
            }
        }
        List<String> files = new ArrayList<>();
        for (String r : roots) collectEcore(new File(r), files);
        Collections.sort(files);
        for (String f : files) for (String ns : nsUris(f)) nsIndex.putIfAbsent(ns, f);
        for (Map.Entry<String, String> a : aliases.entrySet()) {
            Object p = EPackage.Registry.INSTANCE.get(a.getValue());
            if (p != null) EPackage.Registry.INSTANCE.put(a.getKey(), p);
        }
        mmSet = new ResourceSetImpl();
        offline(mmSet);
    }

    static void collectEcore(File d, List<String> out) {
        File[] fs = d.listFiles();
        if (fs == null) return;
        for (File f : fs) {
            if (f.isDirectory()) { if (!f.getName().equals(".git")) collectEcore(f, out); }
            else if (f.getName().endsWith(".ecore")) out.add(f.getPath());
        }
    }

    static final Pattern NS = Pattern.compile("<(?:[A-Za-z_][\\w.-]*:)?(?:EPackage|eSubpackages)\\b[^>]*?\\snsURI=\"([^\"]*)\"");

    /** nsURIs of the packages declared in an .ecore file (root and subpackages). */
    static List<String> nsUris(String file) {
        List<String> out = new ArrayList<>();
        try {
            String text = new String(Files.readAllBytes(Paths.get(file)), StandardCharsets.UTF_8);
            Matcher m = NS.matcher(text);
            while (m.find()) out.add(m.group(1).replace("&amp;", "&"));
        } catch (IOException e) {
            // unreadable: nothing
        }
        return out;
    }

    static void offline(ResourceSet rs) {
        rs.getURIConverter().getURIHandlers().add(0, new URIHandlerImpl() {
            @Override public boolean canHandle(URI uri) { return "http".equals(uri.scheme()) || "https".equals(uri.scheme()); }
            @Override public InputStream createInputStream(URI uri, Map<?, ?> options) throws IOException { throw new IOException("offline: " + uri); }
        });
    }

    /** A package descriptor loading `file` into the shared metamodel resource set. */
    static EPackage.Descriptor descriptor(String ns, String file) {
        return new EPackage.Descriptor() {
            EPackage loaded;
            public EPackage getEPackage() {
                if (loaded != null) return loaded;
                try {
                    Resource r = mmSet.getResource(fileUri(file), true);
                    loaded = find(r.getContents(), ns);
                } catch (RuntimeException e) {
                    loaded = null;
                }
                return loaded;
            }
            public EFactory getEFactory() { EPackage p = getEPackage(); return p == null ? null : p.getEFactoryInstance(); }
        };
    }

    static EPackage find(List<EObject> objs, String ns) {
        for (EObject o : objs) {
            if (o instanceof EPackage) {
                EPackage p = (EPackage) o;
                if (ns.equals(p.getNsURI())) return p;
                EPackage s = find(new ArrayList<>(p.getESubpackages()), ns);
                if (s != null) return s;
            }
        }
        return null;
    }

    /** A resource set for loading `modelPath`: same-directory metamodels first, then the index. */
    static ResourceSet newResourceSet(String modelPath) {
        ResourceSet rs = new ResourceSetImpl();
        offline(rs);
        EPackage.Registry reg = rs.getPackageRegistry();
        File dir = new File(modelPath).getAbsoluteFile().getParentFile();
        File[] local = dir.listFiles((d, n) -> n.endsWith(".ecore"));
        Map<String, String> ns = new LinkedHashMap<>();
        if (local != null) {
            List<String> paths = new ArrayList<>();
            for (File f : local) paths.add(f.getPath());
            Collections.sort(paths);
            for (String p : paths) for (String u : nsUris(p)) ns.putIfAbsent(u, p);
        }
        for (Map.Entry<String, String> e : nsIndex.entrySet()) ns.putIfAbsent(e.getKey(), e.getValue());
        for (Map.Entry<String, String> e : ns.entrySet()) {
            if (EPackage.Registry.INSTANCE.containsKey(e.getKey()) && EPackage.Registry.INSTANCE.get(e.getKey()) instanceof EPackage) continue;
            reg.put(e.getKey(), descriptor(e.getKey(), e.getValue()));
        }
        for (Map.Entry<String, String> a : aliases.entrySet()) {
            Object p = EPackage.Registry.INSTANCE.get(a.getValue());
            if (p != null) reg.put(a.getKey(), p);
        }
        return rs;
    }

    static void models(Path list, Path out) throws IOException {
        Files.createDirectories(out);
        List<String> results = new ArrayList<>();
        int n = 0;
        for (String path : Files.readAllLines(list)) {
            if (path.isBlank() || path.startsWith("#")) continue;
            String id = String.format("%04d", n++);
            String status;
            int errors = 0;
            String saveHash = "-", dumpHash = "-";
            ResourceSet rs = newResourceSet(path);
            Resource r = null;
            try {
                r = rs.getResource(fileUri(path), true);
                status = "ok";
            } catch (RuntimeException e) {
                r = rs.getResource(fileUri(path), false);
                status = "loaderr";
            }
            if (r != null) {
                errors = r.getErrors().size();
                if (errors > 0) {
                    StringBuilder eb = new StringBuilder();
                    for (Resource.Diagnostic d : r.getErrors()) eb.append(d.getLine()).append(": ").append(d.getMessage()).append('\n');
                    Files.write(out.resolve(id + ".errors"), eb.toString().getBytes(StandardCharsets.UTF_8));
                }
                if (r.isLoaded() || !r.getContents().isEmpty()) {
                    try {
                        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                        r.save(bytes, null);
                        Files.write(out.resolve(id + ".xmi"), bytes.toByteArray());
                        saveHash = fnv(bytes.toByteArray());
                    } catch (Exception e) {
                        saveHash = "error";
                    }
                    try {
                        EcoreUtil.resolveAll(rs);
                    } catch (RuntimeException e) {
                        // unresolvable proxies stay proxies
                    }
                    try {
                        String dump = CanonDump.dump(Collections.singletonList(r));
                        byte[] d = dump.getBytes(StandardCharsets.UTF_8);
                        Files.write(out.resolve(id + ".jsonl"), d);
                        dumpHash = fnv(d);
                    } catch (RuntimeException e) {
                        // e.g. dynamic subclasses of Ecore's generated classes
                        dumpHash = "error";
                    }
                }
            } else {
                status = "fail";
            }
            results.add(path + "\t" + status + "\t" + errors + "\t" + saveHash + "\t" + dumpHash);
        }
        Files.write(out.resolve("results.tsv"), (String.join("\n", results) + "\n").getBytes(StandardCharsets.UTF_8));
    }

    /** Saves a copy of the runtime EcorePackage (classifier and feature order as EMF has them). */
    static void saveEcore(String path) throws IOException {
        ResourceSet rs = new ResourceSetImpl();
        Resource r = rs.createResource(fileUri(path));
        r.getContents().add(EcoreUtil.copy(EcorePackage.eINSTANCE));
        r.save(null);
    }

    static String fnv(byte[] b) {
        long h = 0xcbf29ce484222325L;
        for (byte x : b) {
            h ^= (x & 0xff);
            h *= 0x100000001b3L;
        }
        return String.format("%016x", h);
    }

    static boolean compare(String a, String b) {
        ResourceSet ra = newResourceSet(a), rb = newResourceSet(b);
        Resource x = ra.getResource(fileUri(a), true);
        Resource y = rb.getResource(fileUri(b), true);
        return SuiteCompare.equal(x.getContents(), y.getContents());
    }
}
