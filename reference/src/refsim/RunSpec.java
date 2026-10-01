package refsim;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/** One simulation run: model files, seed, stop conditions. */
public final class RunSpec {
    public String name = "run";
    public File usageModel;
    public List<File> allocations = new ArrayList<>();
    public File monitorRepository;
    public long seed = 0;
    /** SimuLizar's max simulation time is a long (SimuComConfig.SIMULATION_TIME); -1 = off. */
    public long maxSimTime = -1;
    /** Max measurements (main measurement count = finished usage scenario runs); -1 = off. */
    public long maxMeasurements = -1;
    /** SimuComConfig.SIMULATE_LINKING_RESOURCES (middleware marshalling; UI default false). */
    public boolean simulateLinkingResources = false;
    /** SimuComConfig.SIMULATE_THROUGHPUT_OF_LINKING_RESOURCES (sum of BYTESIZE; UI default true). */
    public boolean simulateThroughputOfLinkingResources = true;

    /** The 6 MT19937 seed words: seed+0 .. seed+5 (SimuLizar UI default is 0..5, i.e. seed 0). */
    public long[] seedWords() {
        long[] w = new long[6];
        for (int i = 0; i < 6; i++) {
            w[i] = seed + i;
        }
        return w;
    }

    /** Model given as a directory (searched recursively) or a comma separated file list. */
    public void setModel(String spec) throws java.io.IOException {
        for (String part : spec.split(",")) {
            File f = new File(part).getCanonicalFile();
            if (f.isDirectory()) {
                name = f.getName();
                List<File> all = new ArrayList<>();
                collect(f, all);
                for (File x : all) {
                    addFile(x, false);
                }
            } else {
                addFile(f, true);
            }
        }
    }

    private void addFile(File f, boolean explicit) {
        String n = f.getName();
        if (n.endsWith(".usagemodel")) {
            if (usageModel != null && !explicit) {
                throw new IllegalArgumentException("several .usagemodel files; pass them explicitly: " + usageModel
                        + ", " + f);
            }
            usageModel = f;
        } else if (n.endsWith(".allocation")) {
            allocations.add(f);
        } else if (n.endsWith(".monitorrepository")) {
            if (monitorRepository != null && !explicit) {
                throw new IllegalArgumentException("several .monitorrepository files: " + monitorRepository + ", " + f);
            }
            monitorRepository = f;
        } else if (explicit) {
            throw new IllegalArgumentException("unsupported model file " + f);
        }
    }

    private static void collect(File d, List<File> out) {
        File[] fs = d.listFiles();
        if (fs == null) {
            return;
        }
        java.util.Arrays.sort(fs);
        for (File f : fs) {
            if (f.isDirectory()) {
                if (!f.getName().equals("expected")) {
                    collect(f, out);
                }
            } else {
                out.add(f);
            }
        }
    }

    /** Reads corpus/&lt;model&gt;/run.json. */
    @SuppressWarnings("unchecked")
    public static RunSpec fromRunJson(File runJson) throws Exception {
        Map<String, Object> m = (Map<String, Object>) Json.parse(Files.readString(runJson.toPath()));
        File dir = runJson.getCanonicalFile().getParentFile();
        RunSpec r = new RunSpec();
        r.name = dir.getName();
        Object um = m.get("usagemodel");
        if (um != null) {
            r.usageModel = new File(dir, (String) um).getCanonicalFile();
            for (Object a : (List<Object>) m.get("allocation")) {
                r.allocations.add(new File(dir, (String) a).getCanonicalFile());
            }
            Object mon = m.get("monitorrepository");
            if (mon != null) {
                r.monitorRepository = new File(dir, (String) mon).getCanonicalFile();
            }
        } else {
            r.setModel(dir.getPath());
        }
        if (m.get("seed") != null) {
            r.seed = ((Number) m.get("seed")).longValue();
        }
        if (m.get("max_sim_time") != null) {
            r.maxSimTime = ((Number) m.get("max_sim_time")).longValue();
        }
        if (m.get("max_measurements") != null) {
            r.maxMeasurements = ((Number) m.get("max_measurements")).longValue();
        }
        if (m.get("simulate_linking_resources") != null) {
            r.simulateLinkingResources = (Boolean) m.get("simulate_linking_resources");
        }
        if (m.get("simulate_throughput_of_linking_resources") != null) {
            r.simulateThroughputOfLinkingResources = (Boolean) m.get("simulate_throughput_of_linking_resources");
        }
        return r;
    }

    public void validate() {
        if (usageModel == null || allocations.isEmpty()) {
            throw new IllegalArgumentException("model needs a .usagemodel and at least one .allocation");
        }
        if (maxSimTime <= 0 && maxMeasurements <= 0) {
            throw new IllegalArgumentException("need --max-sim-time and/or --max-measurements");
        }
    }
}
