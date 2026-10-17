package simoxide.slingshot.seed;

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Field;
import java.security.Provider;
import java.security.Security;
import java.util.Random;
import java.util.function.DoubleConsumer;

/**
 * Java agent of the SimOxide Slingshot reference runner (reference/slingshot). Loaded with
 * {@code -javaagent:seed-agent.jar} before Equinox starts, so it is in place before the JVM-global
 * random sources are first used. It changes two JVM-global sources and nothing else:
 *
 * <ol>
 * <li><b>Math.random()</b>: the {@code java.util.Random} behind {@code Math.random()} is replaced by
 * {@link TapeRandom}, a plain {@code java.util.Random} subclass (same LCG, same {@code nextDouble})
 * that can be reseeded and optionally reports every draw. Slingshot uses {@code Math.random()} for
 * probabilistic branches ({@code TransitionDeterminer}) and link failures
 * ({@code SimulatedLinkingResource}).</li>
 * <li><b>UUID.randomUUID()</b> (and every other default {@code new SecureRandom()}): a provider with a
 * seedable SplitMix64 {@code SecureRandom} is installed at position 1. Slingshot gives every event, user,
 * job and request a random UUID; several hash-based collections are keyed by them
 * ({@code Job.hashCode}, {@code User.hashCode}), so their iteration order follows the UUIDs.</li>
 * </ol>
 *
 * It always hooks the draw sites of {@code Math.random()} for the tape ({@link SiteTransformer},
 * {@link Sites}); that changes nothing but a static reference. With the agent argument {@code ordered} it also rewrites Slingshot's hash-ordered collections to
 * insertion order ({@link OrderingTransformer}).
 *
 * The runner bundle reaches {@link #reseed(long)} and {@link #reseedMath(long)} through the system class loader by reflection.
 */
public final class SeedAgent {

	static volatile TapeRandom math;
	static volatile boolean installed;
	static volatile OrderingTransformer ordering;
	static volatile SiteTransformer sites;

	private SeedAgent() {
	}

	/**
	 * Agent arguments (comma separated): {@code ordered} adds the {@link OrderingTransformer} for Slingshot's
	 * classes, {@code qvto} extends it to QVTo and OCL.
	 */
	public static void premain(final String args, final Instrumentation inst) throws Exception {
		Security.insertProviderAt(new DetProvider(), 1);
		installMathRandom();
		sites = new SiteTransformer();
		inst.addTransformer(sites);
		final java.util.List<String> a = args == null ? java.util.List.of() : java.util.Arrays.asList(args.split(","));
		if (a.contains("ordered")) {
			ordering = new OrderingTransformer(a.contains("qvto"));
			inst.addTransformer(ordering);
		}
		installed = true;
	}

	/** "off", or the number of rewritten classes and sites so far. */
	public static String ordering() {
		final OrderingTransformer o = ordering;
		return o == null ? "off"
				: (o.qvto ? "slingshot+qvto: " : "slingshot: ") + o.classes.get() + " classes, " + o.sites.get() + " sites";
	}

	@SuppressWarnings("removal")
	private static void installMathRandom() throws Exception {
		final Class<?> holder = Class.forName("java.lang.Math$RandomNumberGeneratorHolder");
		Math.random(); // initialise the holder, then swap its static final field
		final Field f = holder.getDeclaredField("randomNumberGenerator");
		final Field uf = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
		uf.setAccessible(true);
		final sun.misc.Unsafe u = (sun.misc.Unsafe) uf.get(null);
		final TapeRandom r = new TapeRandom();
		u.putObjectVolatile(u.staticFieldBase(f), u.staticFieldOffset(f), r);
		if (!(f.getType().isInstance(r))) {
			throw new IllegalStateException("Math.random holder has an unexpected type");
		}
		math = r;
	}

	/** Number of draw-site hooks inserted so far ({@link SiteTransformer}). */
	public static int siteHooks() {
		final SiteTransformer s = sites;
		return s == null ? 0 : s.hooked;
	}

	/** True when the agent was loaded (premain ran). */
	public static boolean installed() {
		return installed && math != null;
	}

	/** UUIDs (every default {@code SecureRandom}) come from SplitMix64 seeded with {@code seed}. */
	public static void reseed(final long seed) {
		DetSecureRandomSpi.reseed(seed);
	}

	/** {@code Math.random()} becomes the {@code nextDouble()} stream of {@code new java.util.Random(seed)}. */
	public static void reseedMath(final long seed) {
		math.setSeed(seed);
	}

	/** Every {@code Math.random()} result is passed to {@code sink} (null: off). */
	public static void mathSink(final DoubleConsumer sink) {
		math.sink = sink;
	}

	/** {@code Math.random()} draws since the last reseed. */
	public static long mathDraws() {
		return math.draws;
	}

	/** {@code java.util.Random} whose {@code nextDouble()} can report its results. */
	static final class TapeRandom extends Random {
		private static final long serialVersionUID = 1L;
		volatile DoubleConsumer sink;
		long draws;

		@Override
		public synchronized void setSeed(final long seed) {
			super.setSeed(seed);
			draws = 0;
		}

		@Override
		public double nextDouble() {
			final double d = super.nextDouble();
			draws++;
			final DoubleConsumer s = sink;
			if (s != null) {
				s.accept(d);
			}
			return d;
		}
	}

	static final class DetProvider extends Provider {
		private static final long serialVersionUID = 1L;

		DetProvider() {
			super("SimOxideDeterministic", "1.0", "seedable SplitMix64 SecureRandom for reference runs (not secure)");
			putService(new Service(this, "SecureRandom", "SimOxideSplitMix64", DetSecureRandomSpi.class.getName(),
					null, null));
		}
	}
}
