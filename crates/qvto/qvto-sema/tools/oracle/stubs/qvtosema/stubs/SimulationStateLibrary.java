package qvtosema.stubs;

import org.eclipse.m2m.qvt.oml.blackbox.java.Operation;
import org.eclipse.m2m.qvt.oml.blackbox.java.Operation.Kind;

/**
 * Stub of SimuLizar's org.palladiosimulator.simulizar.action.repository.black.SimulationStateLibrary
 * (EPL-2.0), registered under that unit name by the compile-oracle cases. Fails if called.
 */
public class SimulationStateLibrary {
	public SimulationStateLibrary() {
	}

	@Operation(kind = Kind.HELPER)
	public static double getSimulationTime() {
		throw new UnsupportedOperationException("qvto-rs compile oracle: SimulationStateLibrary stub");
	}
}
