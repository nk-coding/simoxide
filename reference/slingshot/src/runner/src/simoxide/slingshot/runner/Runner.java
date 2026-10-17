package simoxide.slingshot.runner;

import java.io.File;
import java.io.IOException;
import java.io.Writer;
import java.lang.management.ManagementFactory;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.TreeMap;
import java.util.function.DoubleConsumer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import javax.measure.Measure;
import javax.measure.quantity.Quantity;

import org.apache.log4j.Level;
import org.apache.log4j.Logger;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.resource.Resource;
import org.palladiosimulator.analyzer.slingshot.core.Slingshot;
import org.palladiosimulator.analyzer.slingshot.core.api.SimulationDriver;
import org.palladiosimulator.analyzer.slingshot.core.extension.PCMResourceSetPartitionProvider;
import org.palladiosimulator.analyzer.slingshot.workflow.WorkflowConfigurationModule;
import org.palladiosimulator.analyzer.workflow.core.ConstantsContainer;
import org.palladiosimulator.analyzer.workflow.core.blackboard.PCMResourceSetPartition;
import org.palladiosimulator.analyzer.workflow.core.jobs.LoadModelIntoBlackboardJob;
import org.palladiosimulator.analyzer.workflow.core.jobs.PreparePCMBlackboardPartitionJob;
import org.palladiosimulator.edp2.dao.MeasurementsDao;
import org.palladiosimulator.edp2.impl.RepositoryManager;
import org.palladiosimulator.edp2.models.ExperimentData.DataSeries;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentGroup;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentRun;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentSetting;
import org.palladiosimulator.edp2.models.ExperimentData.Measurement;
import org.palladiosimulator.edp2.models.ExperimentData.MeasurementRange;
import org.palladiosimulator.edp2.models.ExperimentData.RawMeasurements;
import org.palladiosimulator.edp2.models.Repository.LocalMemoryRepository;
import org.palladiosimulator.edp2.models.Repository.RepositoryFactory;
import org.palladiosimulator.edp2.util.MeasurementsUtility;
import org.palladiosimulator.metricspec.MetricDescription;

import de.uka.ipd.sdq.probfunction.math.IRandomGenerator;
import de.uka.ipd.sdq.probfunction.math.impl.ProbabilityFunctionFactoryImpl;
import de.uka.ipd.sdq.simucomframework.core.SimuComConfig;
import de.uka.ipd.sdq.workflow.mdsd.blackboard.MDSDBlackboard;
import simoxide.slingshot.instrument.Instrument;
import simoxide.slingshot.instrument.Json;
import simoxide.slingshot.instrument.Keys;

/**
 * Headless Slingshot run(s) of one model directory. Derived from the bench runner in
 * old/palladio-research/work-slingshot/headless-app (itself after SlingshotTestRun of
 * Palladio-Analyzer-Slingshot-E2E-Tests). See docs/reference-simulator/slingshot.md.
 *
 * <pre>
 * -model DIR          model directory (all files with a known extension are loaded; .slo is skipped)
 * -out DIR            output directory (one subdirectory run-N per run when -runs &gt; 1)
 * -simTime T          simulated time in s (Slingshot's only stop condition)       [1000]
 * -seed N             seed all random sources (omit: unseeded, as the nightly)     [none]
 * -stoex mt|jur       StoEx generator when seeded: SimuCom's MT19937 {seed..seed+5} (the stream
 *                     SimuLizar/refsim/SimOxide use for the same seed) or java.util.Random(seed)
 *                     (the type of the nightly's unseeded DefaultRandomGenerator)          [mt]
 * -runs N             runs in this JVM (warm repeats with the same seed)          [1]
 * -instrument B       trace/tape/SPD log on (false: none of the extension is registered) [true]
 * -trace B -tape B -spd B   individual outputs                                   [true]
 * -spdLevel decisions|all   SPD evaluations rejected by the target-group check are logged only with all
 * -measurements B     measurements.csv from the EDP2 repository                   [true]
 * -saveModels B       write the (possibly adjusted) models after the run to models/  [false]
 * -linkingResources B SimuComConfig "simulate linking resources" (default: run.json, else false)
 * -log LEVEL          log4j root level                                            [WARN]
 * </pre>
 */
public final class Runner {

	static final List<String> EXTENSIONS = List.of("repository", "system", "resourceenvironment", "allocation",
			"usagemodel", "monitorrepository", "spd", "semanticspd");
	static final long MATH_SEED_OFFSET = 1_000_003L;

	private Runner() {
	}

	public static int main(final String[] args) throws Exception {
		final Map<String, String> a = new HashMap<>();
		for (int i = 0; i + 1 < args.length; i += 2) {
			a.put(args[i], args[i + 1]);
		}
		Logger.getRootLogger().setLevel(Level.toLevel(a.getOrDefault("-log", "WARN")));
		final long uptimeAtStart = ManagementFactory.getRuntimeMXBean().getUptime();

		final File dir = new File(a.get("-model"));
		final Path out = Path.of(a.getOrDefault("-out", "slingshot-out")).toAbsolutePath();
		final double simTime = Double.parseDouble(a.getOrDefault("-simTime", "1000"));
		final int runs = Integer.parseInt(a.getOrDefault("-runs", "1"));
		final Long seed = a.containsKey("-seed") ? Long.valueOf(a.get("-seed")) : null;
		final boolean instrument = bool(a, "-instrument", true);
		final Map<String, String> runJson = readRunJson(new File(dir, "run.json"));
		final boolean links = a.containsKey("-linkingResources") ? bool(a, "-linkingResources", false)
				: Boolean.parseBoolean(runJson.getOrDefault("simulate_linking_resources", "false"));

		final List<URI> models = modelUris(dir);
		Files.createDirectories(out);
		final Seeding seeding = seed == null ? null : new Seeding(a.getOrDefault("-stoex", "mt"));
		final String ordering = agentOrdering();
		System.out.printf(Locale.ROOT, "[slingshot] %d model files from %s; seed=%s stoex=%s ordering=%s%n",
				models.size(), dir, seed, seeding == null ? "-" : seeding.stoex, ordering);

		final StringBuilder timings = new StringBuilder(
				"run,jvm,load_ms,init_ms,sim_ms,collect_ms,wall_ms,events,measurements,sim_time\n");
		int failures = 0;
		for (int i = 0; i < runs; i++) {
			final Path rd = runs == 1 ? out : out.resolve("run-" + i);
			Files.createDirectories(rd);
			final long t0 = System.nanoTime();
			final Result r = run(models, rd, simTime, seed, seeding, links, instrument, a);
			final double wall = (System.nanoTime() - t0) / 1e6;
			timings.append(String.format(Locale.ROOT, "%d,%s,%.1f,%.1f,%.1f,%.1f,%.1f,%d,%d,%s%n", i,
					i == 0 ? "cold" : "warm", r.loadMs, r.initMs, r.simMs, r.collectMs, wall, r.events, r.rows,
					Json.num(simTime)));
			System.out.printf(Locale.ROOT,
					"run %d: load=%.0fms init=%.0fms sim=%.0fms collect=%.0fms wall=%.0fms events=%d rows=%d %s%n", i,
					r.loadMs, r.initMs, r.simMs, r.collectMs, wall, r.events, r.rows, r.instrumentSummary);
			if (r.error != null) {
				failures++;
				System.out.println("   ERROR: " + r.error);
			}
		}
		Files.writeString(out.resolve("timings.csv"), timings.toString());
		System.out.printf(Locale.ROOT, "[slingshot] ordering rule: %s%n", agentOrdering());
		System.out.printf(Locale.ROOT, "[slingshot] %d runs; JVM uptime at start %d ms, at end %d ms%n", runs,
				uptimeAtStart, ManagementFactory.getRuntimeMXBean().getUptime());
		return failures == 0 ? 0 : 1;
	}

	/** The seed agent's collection-ordering rule: "off", "no agent", or rewrite counts. */
	static String agentOrdering() {
		try {
			return (String) ClassLoader.getSystemClassLoader().loadClass("simoxide.slingshot.seed.SeedAgent")
					.getMethod("ordering").invoke(null);
		} catch (final ReflectiveOperationException e) {
			return "no agent";
		}
	}

	static boolean bool(final Map<String, String> a, final String k, final boolean d) {
		return a.containsKey(k) ? Boolean.parseBoolean(a.get(k)) : d;
	}

	static final class Result {
		double loadMs, initMs, simMs, collectMs;
		int events;
		long rows;
		String instrumentSummary = "";
		String error;
	}

	/** Model files of a directory in name order; .slo (SLO metamodel not installed) is skipped. */
	public static List<URI> modelUris(final File dir) {
		final File[] files = dir.listFiles();
		if (files == null) {
			throw new IllegalArgumentException("not a directory: " + dir);
		}
		Arrays.sort(files);
		final List<URI> uris = new ArrayList<>();
		for (final File f : files) {
			final String n = f.getName();
			final int i = n.lastIndexOf('.');
			if (i > 0 && EXTENSIONS.contains(n.substring(i + 1))) {
				uris.add(URI.createFileURI(f.getAbsolutePath()));
			}
		}
		return uris;
	}

	static Map<String, String> readRunJson(final File f) throws IOException {
		final Map<String, String> m = new HashMap<>();
		if (f.isFile()) {
			final Matcher mt = Pattern.compile("\"([a-z_]+)\"\\s*:\\s*([^,}\\s]+)").matcher(Files.readString(f.toPath()));
			while (mt.find()) {
				m.put(mt.group(1), mt.group(2).replace("\"", ""));
			}
		}
		return m;
	}

	// ---------------------------------------------------------------------------------------------
	// seeding

	/** Seeds the random sources; see docs/reference-simulator/slingshot.md "Seeding". */
	static final class Seeding {
		final String stoex;
		final Class<?> agent;

		Seeding(final String stoex) {
			if (!stoex.equals("jur") && !stoex.equals("mt")) {
				throw new IllegalArgumentException("-stoex jur|mt");
			}
			this.stoex = stoex;
			Class<?> c = null;
			try {
				c = ClassLoader.getSystemClassLoader().loadClass("simoxide.slingshot.seed.SeedAgent");
				if (!(Boolean) c.getMethod("installed").invoke(null)) {
					c = null;
				}
			} catch (final ReflectiveOperationException e) {
				c = null;
			}
			if (c == null) {
				throw new IllegalStateException("-seed needs the seed agent (-javaagent:seed-agent.jar); run via run.sh");
			}
			this.agent = c;
		}

		void apply(final long seed, final SimuComConfig config) throws Exception {
			final IRandomGenerator base;
			if (stoex.equals("mt")) {
				base = config.getRandomGenerator(); // SimuCom MT19937 seeded {seed, ..., seed+5}
			} else {
				final Random r = new Random(seed);
				base = new IRandomGenerator() {
					@Override
					public double random() {
						return r.nextDouble();
					}

					@Override
					public void dispose() {
					}
				};
			}
			ProbabilityFunctionFactoryImpl.getInstance().setRandomGenerator(new IRandomGenerator() {
				@Override
				public double random() {
					final double u = base.random();
					Instrument.tape("stoex", u);
					return u;
				}

				@Override
				public void dispose() {
					base.dispose();
				}
			});
			agent.getMethod("reseed", long.class).invoke(null, seed);
			final Method sink = agent.getMethod("mathSink", DoubleConsumer.class);
			sink.invoke(null, Instrument.active() ? (DoubleConsumer) u -> Instrument.tape("math", u) : null);
			// Math.random gets its own seed: java.util.Random(seed) would repeat the StoEx stream
			agent.getMethod("reseedMath", long.class).invoke(null, seed + MATH_SEED_OFFSET);
		}
	}

	// ---------------------------------------------------------------------------------------------

	static Result run(final List<URI> models, final Path rd, final double simTime, final Long seed,
			final Seeding seeding, final boolean links, final boolean instrument, final Map<String, String> a)
			throws Exception {
		final Result r = new Result();
		long t = System.nanoTime();

		final MDSDBlackboard blackboard = new MDSDBlackboard();
		final PreparePCMBlackboardPartitionJob prep = new PreparePCMBlackboardPartitionJob();
		prep.setBlackboard(blackboard);
		prep.execute(new NullProgressMonitor());
		for (final URI uri : models) {
			final LoadModelIntoBlackboardJob job = new LoadModelIntoBlackboardJob(uri,
					ConstantsContainer.DEFAULT_PCM_INSTANCE_PARTITION_ID);
			job.setBlackboard(blackboard);
			job.execute(new NullProgressMonitor());
		}
		final PCMResourceSetPartition partition = (PCMResourceSetPartition) blackboard
				.getPartition(ConstantsContainer.DEFAULT_PCM_INSTANCE_PARTITION_ID);
		partition.resolveAllProxies();
		Keys.inputIds = Keys.collectIds(partition.getResourceSet());

		final Map<String, Object> cfg = new HashMap<>();
		cfg.put(SimuComConfig.VERBOSE_LOGGING, false);
		cfg.put(SimuComConfig.SIMULATOR_ID, SimuComConfig.DEFAULT_SIMULATOR_ID);
		cfg.put(SimuComConfig.USE_FIXED_SEED, seed != null);
		if (seed != null) {
			for (int i = 0; i < 6; i++) {
				cfg.put(SimuComConfig.FIXED_SEED_PREFIX + i, String.valueOf(seed + i));
			}
		}
		cfg.put(SimuComConfig.SIMULATION_TIME, String.valueOf((long) simTime));
		cfg.put(SimuComConfig.MAXIMUM_MEASUREMENT_COUNT, String.valueOf(Long.MAX_VALUE));
		cfg.put(SimuComConfig.VARIATION_ID, SimuComConfig.DEFAULT_VARIATION_NAME);
		cfg.put(SimuComConfig.EXPERIMENT_RUN, SimuComConfig.DEFAULT_EXPERIMENT_RUN);
		cfg.put(SimuComConfig.SIMULATE_LINKING_RESOURCES, links);
		cfg.put(SimuComConfig.SIMULATE_FAILURES, false);
		cfg.put(SimuComConfig.PERSISTENCE_RECORDER_NAME, "Experiment Data Persistency & Presentation (EDP2)");
		final LocalMemoryRepository repo = RepositoryFactory.eINSTANCE.createLocalMemoryRepository();
		RepositoryManager.addRepository(RepositoryManager.getCentralRepository(), repo);
		cfg.put("EDP2RepositoryID", repo.getId());
		final SimuComConfig config = new SimuComConfig(cfg, false);

		if (instrument) {
			final Instrument.Options o = new Instrument.Options();
			o.dir = rd;
			o.trace = bool(a, "-trace", true);
			o.tape = bool(a, "-tape", true);
			o.spd = bool(a, "-spd", true);
			o.spdLevel = a.getOrDefault("-spdLevel", "decisions");
			o.header = new Json().str("simulator", "slingshot").str("seed", seed == null ? "none" : seed.toString())
					.str("stoex", seeding == null ? "unseeded" : seeding.stoex)
					.bool("ordered", !agentOrdering().equals("off") && !agentOrdering().equals("no agent"))
					.num("sim_time", simTime)
					.bool("linking_resources", links).end();
			Instrument.begin(o);
		}
		if (seeding != null) {
			seeding.apply(seed, config);
		}
		r.loadMs = (System.nanoTime() - t) / 1e6;

		t = System.nanoTime();
		final SimulationDriver driver = Slingshot.getInstance().getSimulationDriver();
		WorkflowConfigurationModule.blackboardProvider.set(blackboard);
		WorkflowConfigurationModule.simuComConfigProvider.set(config);
		Slingshot.getInstance().getInstance(PCMResourceSetPartitionProvider.class).set(partition);
		driver.init(config, new NullProgressMonitor());
		r.initMs = (System.nanoTime() - t) / 1e6;
		t = System.nanoTime();
		try {
			driver.start(); // blocks until SimulationFinished
		} catch (final Throwable e) {
			r.error = e.toString();
		}
		r.simMs = (System.nanoTime() - t) / 1e6;

		t = System.nanoTime();
		if (instrument) {
			r.instrumentSummary = Instrument.end();
		}
		try {
			final java.lang.reflect.Field ef = driver.getClass().getDeclaredField("engine");
			ef.setAccessible(true);
			r.events = ((org.palladiosimulator.analyzer.slingshot.core.api.SimulationInformation) ef.get(driver))
					.consumedEvents();
		} catch (final Exception e) {
			r.events = -1;
		}
		if (bool(a, "-measurements", true)) {
			r.rows = writeMeasurements(repo, rd.resolve("measurements.csv"));
		}
		if (bool(a, "-saveModels", false)) {
			saveModels(partition, rd.resolve("models"));
		}
		RepositoryManager.removeRepository(RepositoryManager.getCentralRepository(), repo);
		if (seeding != null) {
			seeding.agent.getMethod("mathSink", DoubleConsumer.class).invoke(null, (Object) null);
		}
		r.collectMs = (System.nanoTime() - t) / 1e6;
		Files.writeString(rd.resolve("summary.json"),
				new Json().integer("events", r.events).integer("rows", r.rows).raw("instrument",
						r.instrumentSummary.isEmpty() ? "null" : r.instrumentSummary).str("error", r.error).end()
						+ "\n");
		return r;
	}

	// ---------------------------------------------------------------------------------------------
	// outputs

	/** EDP2 → measurements.csv (docs/guide/formats.md §4): series sorted by key, rows in EDP2 order. */
	static long writeMeasurements(final LocalMemoryRepository repo, final Path file) throws IOException {
		final Map<String, List<double[]>> series = new LinkedHashMap<>();
		for (final ExperimentGroup g : repo.getExperimentGroups()) {
			for (final ExperimentSetting s : g.getExperimentSettings()) {
				for (final ExperimentRun run : s.getExperimentRuns()) {
					for (final Measurement m : run.getMeasurement()) {
						final MetricDescription metric = m.getMeasuringType().getMetric();
						final String key = Keys.csvField(Keys.mpKey(m.getMeasuringType().getMeasuringPoint())) + ","
								+ Keys.csvField(metric.getName());
						final List<double[]> rows = series.computeIfAbsent(key, k -> new ArrayList<>());
						final int ti = Keys.timeIndex(metric);
						for (final MeasurementRange range : m.getMeasurementRanges()) {
							final RawMeasurements raw = range.getRawMeasurements();
							if (raw == null) {
								continue;
							}
							final List<List<Measure<Object, Quantity>>> cols = new ArrayList<>();
							int n = Integer.MAX_VALUE;
							for (final DataSeries ds : raw.getDataSeries()) {
								@SuppressWarnings("unchecked")
								final MeasurementsDao<Object, Quantity> dao = (MeasurementsDao<Object, Quantity>) (MeasurementsDao<?, ?>) MeasurementsUtility
										.getMeasurementsDao(ds);
								final List<Measure<Object, Quantity>> l = dao.getMeasurements();
								cols.add(l);
								n = Math.min(n, l.size());
							}
							if (cols.isEmpty()) {
								continue;
							}
							for (int i = 0; i < n; i++) {
								final double[] row = new double[cols.size()];
								for (int c = 0, k = 1; c < cols.size(); c++) {
									final int pos = c == ti ? 0 : (ti < 0 ? c : k++);
									row[pos] = value(cols.get(c).get(i));
								}
								rows.add(row);
							}
						}
					}
				}
			}
		}
		long total = 0;
		try (Writer w = Files.newBufferedWriter(file, StandardCharsets.UTF_8)) {
			w.write("measuring_point,metric,time,value\n");
			final StringBuilder sb = new StringBuilder();
			for (final Map.Entry<String, List<double[]>> e : new TreeMap<>(series).entrySet()) {
				for (final double[] row : e.getValue()) {
					sb.setLength(0);
					sb.append(e.getKey());
					for (final double v : row) {
						sb.append(',');
						Json.num(sb, v);
					}
					sb.append('\n');
					w.write(sb.toString());
					total++;
				}
			}
		}
		return total;
	}

	private static double value(final Measure<Object, Quantity> me) {
		final Object v = me.getValue();
		return v instanceof final Number n ? n.doubleValue() : Double.NaN;
	}

	/** Saves the model resources of the run (after any SPD adjustment) into {@code dir}. */
	static void saveModels(final PCMResourceSetPartition partition, final Path dir) throws IOException {
		Files.createDirectories(dir);
		final List<Resource> own = new ArrayList<>();
		for (final Resource res : partition.getResourceSet().getResources()) {
			final URI u = res.getURI();
			if (u != null && u.isFile() && EXTENSIONS.contains(u.fileExtension())) {
				own.add(res);
			}
		}
		for (final Resource res : own) {
			res.setURI(URI.createFileURI(dir.resolve(res.getURI().lastSegment()).toString()));
		}
		for (final Resource res : own) {
			res.save(null);
		}
	}
}
