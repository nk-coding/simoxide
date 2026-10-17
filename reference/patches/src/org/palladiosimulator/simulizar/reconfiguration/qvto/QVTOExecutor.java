package org.palladiosimulator.simulizar.reconfiguration.qvto;

import java.util.List;
import java.util.Map;

import org.palladiosimulator.simulizar.reconfiguration.qvto.util.ModelTransformationCache;
import org.palladiosimulator.simulizar.reconfiguration.qvto.util.QVToModelCache;

import de.uka.ipd.sdq.scheduler.resources.active.IResourceTableManager;

/**
 * QVTo executor helper class that supports executing QVTo reconfiguration rules.
 * 
 * @author Matthias Becker
 * @author Sebastian Lehrig
 * @author Florian Rosenthal
 */
public class QVTOExecutor extends AbstractQVTOExecutor {

    /**
     * Initializes a new instance of the {@link QVTOExecutor} class.
     * 
     * @param knownTransformations
     *            An {@link TransformationCache} which contains all transformation that can be
     *            executed by this instance, might be empty.
     * @param knownModels
     *            A {@link QVToModelCache} that contains all models that can serve as a
     *            transformation parameter.
     */
    public QVTOExecutor(ModelTransformationCache knownTransformations, QVToModelCache knownModels) {
        super(knownTransformations, knownModels);
    }

    public boolean executeTransformations(List<QvtoModelTransformation> transformations,
            IResourceTableManager resourceTableManager, Map<String, Object> configParams) {
        boolean result = true;
        for (QvtoModelTransformation transformation : transformations) {
            // REFSIM (patch P10, DESIGN-SPD R2): new Identifiers draw seeded ids while a rule runs
            refsim.Ids.open();
            try {
                result &= executeTransformation(transformation, resourceTableManager, configParams);
            } finally {
                refsim.Ids.close();
            }
        }
        return result;
    }
}
