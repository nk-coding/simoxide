package simoxide.slingshot.seed;

/**
 * Draw sites of {@code Math.random()} for the tape (instrumentation only). {@link SiteTransformer} makes the
 * two Slingshot methods that draw from {@code Math.random()} announce their site before they draw:
 * <ul>
 * <li>{@code TransitionDeterminer.determineBranchTransition(EList)} (usage-model Branch) and
 * {@code determineProbabilisticBranchTransition(EList)} (SEFF BranchAction) store the transition list; its
 * container is the branch;</li>
 * <li>{@code SimulatedLinkingResource.onJobProgressed(JobProgressed)} stores the linking resource (the
 * failure draw).</li>
 * </ul>
 * The tape sink takes the pending site with the next draw ({@link #take()}). Nothing else reads it, so the
 * hooks cannot change what Slingshot does. Lives on the boot class path and is boot-delegated by Equinox.
 */
public final class Sites {
	private static volatile Object site;

	private Sites() {
	}

	/** Called at the entry of a drawing method (inserted by {@link SiteTransformer}). */
	public static void set(final Object o) {
		site = o;
	}

	/** The pending site, or null; clears it. */
	public static Object take() {
		final Object s = site;
		site = null;
		return s;
	}
}
