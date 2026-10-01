package refsim.corpus;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.resource.Resource;
import org.eclipse.emf.ecore.resource.ResourceSet;
import org.eclipse.emf.ecore.resource.impl.ResourceSetImpl;
import org.eclipse.emf.ecore.util.EcoreUtil;
import org.palladiosimulator.pcm.allocation.Allocation;
import org.palladiosimulator.pcm.usagemodel.UsageModel;

import refsim.RunSpec;

/**
 * Corpus maintenance:
 *
 * <pre>
 * refsim import &lt;src dir | files,...&gt; &lt;destDir&gt; [--keep-monitors] [--triggers] [--run-json '{...}']
 *     copies usage model + allocation(s) and everything they reference (except pathmap models) into
 *     destDir (flat, relative hrefs), adds the refsim default monitor repository (or copies the
 *     model's own with --keep-monitors) and writes run.json if missing.
 * refsim gen &lt;corpusDir&gt; [--only a,b]   (re)generates the hand-made models (see HandMade)
 * </pre>
 */
public final class CorpusTool {
    private CorpusTool() {
    }

    public static int importModel(String[] a) throws Exception {
        String src = a[0];
        File dest = new File(a[1]).getCanonicalFile();
        boolean keepMonitors = false;
        String runJson = HandMade.runJson(1, 100, 0, true);
        for (int i = 2; i < a.length; i++) {
            switch (a[i]) {
            case "--keep-monitors":
                keepMonitors = true;
                break;
            case "--skip-external-calls":
                Monitors.skipExternalCalls = true;
                break;
            case "--triggers":
                // default monitors with triggersSelfAdaptations = true (the EMF default)
                Monitors.triggers = (label, metric) -> true;
                break;
            case "--run-json":
                runJson = a[++i] + "\n";
                break;
            default:
                throw new IllegalArgumentException(a[i]);
            }
        }
        RunSpec spec = new RunSpec();
        spec.setModel(src);
        ResourceSet rs = new ResourceSetImpl();
        UsageModel um = (UsageModel) load(rs, spec.usageModel);
        List<Allocation> allocs = new ArrayList<>();
        for (File f : spec.allocations) {
            allocs.add((Allocation) load(rs, f));
        }
        if (keepMonitors && spec.monitorRepository != null) {
            load(rs, spec.monitorRepository);
        }
        EcoreUtil.resolveAll(rs);
        for (Resource r : rs.getResources()) {
            if (!r.getErrors().isEmpty()) {
                throw new IllegalStateException("errors loading " + r.getURI() + ": " + r.getErrors());
            }
        }
        dest.mkdirs();
        // relocate all non-pathmap, non-plugin resources into dest (flat); keep basenames, disambiguate clashes
        Set<String> used = new HashSet<>();
        Map<Resource, URI> moves = new HashMap<>();
        for (Resource r : new ArrayList<>(rs.getResources())) {
            URI u = r.getURI();
            if ("pathmap".equals(u.scheme()) || u.isPlatformPlugin() || r.getContents().isEmpty()) {
                continue;
            }
            String base = u.lastSegment();
            if (base.endsWith(".metricspec") || base.endsWith(".resourcetype")) {
                continue;
            }
            String nb = base;
            for (int k = 2; !used.add(nb); k++) {
                nb = base.replaceFirst("(\\.[^.]+)$", "_" + k + "$1");
            }
            moves.put(r, URI.createFileURI(new File(dest, nb).getAbsolutePath()));
        }
        for (Map.Entry<Resource, URI> e : moves.entrySet()) {
            e.getKey().setURI(e.getValue());
        }
        if (!keepMonitors || spec.monitorRepository == null) {
            Monitors.addDefaultMonitors(rs, um, allocs.get(0), dest, "refsim");
        }
        for (Resource r : rs.getResources()) {
            if (r.getURI().isFile() && r.getURI().toFileString().startsWith(dest.getPath())) {
                r.save(Monitors.SAVE_OPTIONS);
            }
        }
        File rj = new File(dest, "run.json");
        if (!rj.exists()) {
            Files.writeString(rj.toPath(), runJson);
        }
        // run.json must name the files explicitly when several usage models/allocations were copied
        System.err.println("[refsim] imported " + moves.size() + " model files into " + dest);
        return 0;
    }

    static org.eclipse.emf.ecore.EObject load(ResourceSet rs, File f) throws java.io.IOException {
        Resource r = rs.getResource(URI.createFileURI(f.getCanonicalPath()), true);
        return r.getContents().get(0);
    }
}
