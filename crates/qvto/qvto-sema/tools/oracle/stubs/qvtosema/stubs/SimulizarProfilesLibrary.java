package qvtosema.stubs;

import java.util.List;
import java.util.Set;

import org.eclipse.emf.ecore.EObject;
import org.eclipse.m2m.qvt.oml.blackbox.java.Operation;
import org.eclipse.m2m.qvt.oml.blackbox.java.Operation.Kind;

/**
 * Stub of SimuLizar's org.palladiosimulator.simulizar.action.repository.black.ProfilesLibrary
 * (EPL-2.0), registered under that unit name by the compile-oracle cases: the operations and
 * annotations of the original, with the generated PCM types (Entity, Identifier, NamedElement),
 * which dynamic EMF has no Java classes for, replaced by EObject (the Entity and Identifier
 * overloads become one). Every operation fails if called.
 */
public class SimulizarProfilesLibrary {
	public SimulizarProfilesLibrary() {
	}

	private static RuntimeException stub() {
		return new UnsupportedOperationException("qvto-rs compile oracle: ProfilesLibrary stub");
	}

	@Operation(kind = Kind.QUERY, contextual = true)
	public static boolean hasAppliedStereotype(EObject pcmEntity, String stereotypeName) {
		throw stub();
	}

	public static boolean hasAppliedStereotype(Set<EObject> pcmEntitySet, String stereotypeName) {
		throw stub();
	}

	public static boolean appliedStereotypesEqualsOne(Set<EObject> pcmEntitySet, String stereotypeName) {
		throw stub();
	}

	@Operation(kind = Kind.HELPER, contextual = true)
	public static void applyStereotype(EObject pcmEntity, String stereotypeName) {
		throw stub();
	}

	@Operation(kind = Kind.HELPER, contextual = true)
	public static void removeStereotypeApplications(EObject pcmEntity, String stereotypeName) {
		throw stub();
	}

	public static void setTaggedValue(EObject pcmEntity, int value, String stereotypeName, String taggedValueName) {
		throw stub();
	}

	public static int getIntTaggedValue(EObject pcmEntity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static double getDoubleTaggedValue(EObject pcmEntity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static void delete(List<EObject> rootEObjects, EObject eObject) {
		throw stub();
	}
}
