package simoxide.slingshot.seed;

import java.security.SecureRandomSpi;

/**
 * SplitMix64 behind every {@code SecureRandom} instance: one JVM-global state, so reseeding also
 * affects instances that already exist (such as {@code UUID}'s holder). Explicit seeds from callers are
 * ignored. Until the first {@link #reseed(long)} the state comes from {@code System.nanoTime()}.
 * NOT a secure generator; only for reproducible simulation runs.
 */
public final class DetSecureRandomSpi extends SecureRandomSpi {
	private static final long serialVersionUID = 1L;
	private static long state = System.nanoTime() ^ 0x5DEECE66DL;

	public DetSecureRandomSpi() {
	}

	static synchronized void reseed(final long seed) {
		state = seed;
	}

	private static synchronized long next() {
		long z = (state += 0x9E3779B97F4A7C15L);
		z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
		z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
		return z ^ (z >>> 31);
	}

	@Override
	protected void engineSetSeed(final byte[] seed) {
		// ignored: the stream is controlled by reseed() only
	}

	@Override
	protected void engineNextBytes(final byte[] bytes) {
		int i = 0;
		while (i < bytes.length) {
			long v = next();
			for (int k = 0; k < 8 && i < bytes.length; k++, v >>>= 8) {
				bytes[i++] = (byte) v;
			}
		}
	}

	@Override
	protected byte[] engineGenerateSeed(final int numBytes) {
		final byte[] b = new byte[numBytes];
		engineNextBytes(b);
		return b;
	}
}
